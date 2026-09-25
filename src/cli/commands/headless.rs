use indicatif::{ProgressBar, ProgressStyle};
use crate::core::config::Config;
use crate::core::constants::PROGRESS_UPDATE_INTERVAL_MS;
use crate::core::snapshot::StatsSnapshot;
use crate::runner::LoadEngine;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Run in headless (non-TUI) mode with text progress bar.
pub async fn run_headless(cfg: Config) -> anyhow::Result<()> {
    // Print banner
    println!("{}", crate::tui::banner());
    println!("  URL:          {}", cfg.url);
    println!("  Method:       {}", cfg.method);
    println!("  Mode:         {:?}", cfg.mode);
    if cfg.mode == crate::core::config::Mode::Rps {
        println!("  Target RPS:   {}", cfg.target_rps);
    } else {
        println!("  Users:        {}", cfg.num_users);
    }
    println!("  Duration:     {}s", cfg.steady_dur_secs);
    if cfg.ramp_up_secs > 0 {
        println!("  Ramp Up:      {}s", cfg.ramp_up_secs);
    }
    if cfg.ramp_down_secs > 0 {
        println!("  Ramp Down:    {}s", cfg.ramp_down_secs);
    }
    println!();

    let (tx, rx) = mpsc::unbounded_channel();
    let engine = LoadEngine::new(cfg.clone(), tx)?;
    let stats = Arc::clone(engine.stats());

    let cancel = CancellationToken::new();
    let engine_cancel = cancel.clone();

    // Run engine in background
    let engine_handle = tokio::spawn(async move {
        engine.run(engine_cancel).await;
    });

    let total_dur = cfg.total_duration();
    let start = Instant::now();

    // Progress bar
    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos:>3}% {msg}")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏  "),
    );

    // Monitor loop
    let mut stats_rx = rx;
    let mut last_snap = StatsSnapshot::default();

    loop {
        let elapsed = start.elapsed();
        let pct = if total_dur.is_zero() {
            0.0
        } else {
            (elapsed.as_secs_f64() / total_dur.as_secs_f64() * 100.0).min(100.0)
        };

        // Try to receive stats
        match stats_rx.try_recv() {
            Ok(snap) => {
                last_snap = snap;
                let avg_rps = if elapsed.as_secs_f64() > 0.0 {
                    last_snap.requests as f64 / elapsed.as_secs_f64()
                } else {
                    0.0
                };
                pb.set_position(pct as u64);
                pb.set_message(format!(
                    "  Req: {} | RPS: {:.0} | P99: {:.0}ms | Err: {}",
                    last_snap.requests,
                    avg_rps,
                    last_snap.p99_service_ms,
                    last_snap.fail,
                ));
            }
            Err(_) => {
                pb.set_position(pct as u64);
            }
        }

        if elapsed >= total_dur {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(PROGRESS_UPDATE_INTERVAL_MS)).await;
    }

    // Wait for engine to drain
    cancel.cancel();
    let _ = engine_handle.await;

    pb.finish_with_message("Complete");
    println!();

    // Print summary
    let results = stats.get_results();
    print_summary(&last_snap);

    // Auto-export if --out specified
    if let Some(ref prefix) = cfg.out_prefix {
        export_reports(&results, prefix)?;
    }

    Ok(())
}

fn print_summary(snap: &StatsSnapshot) {
    let total = snap.requests;
    let success = snap.success;
    let fail = snap.fail;

    println!();
    println!("{}", "═".repeat(60));
    println!("  RESULTS");
    println!("{}", "═".repeat(60));
    println!("  Requests:       {}", total);
    println!("  Success:        {}", success);
    println!("  Failed:         {}", fail);
    if total > 0 {
        println!("  Success Rate:   {:.1}%", success as f64 / total as f64 * 100.0);
    }
    println!("  P50 Latency:    {:.1}ms", snap.p50_service_ms);
    println!("  P90 Latency:    {:.1}ms", snap.p90_service_ms);
    println!("  P95 Latency:    {:.1}ms", snap.p95_service_ms);
    println!("  P99 Latency:    {:.1}ms", snap.p99_service_ms);
    println!("  Max Latency:    {:.1}ms", snap.max_service_ms);
    println!("  Avg Queue Wait: {:.1}ms", snap.avg_queue_wait_ms);

    // A non-zero drop count means the generator, not the target, was the
    // bottleneck. The latency figures above do not describe the target, so this
    // must not be buried in a report file.
    if snap.dropped_scheduled > 0 {
        let scheduled = total + snap.dropped_scheduled;
        println!();
        println!("  ⚠ MEASUREMENT INVALID");
        println!("    {} of {} scheduled requests were shed because the", snap.dropped_scheduled, scheduled);
        println!("    concurrency ceiling was saturated. The generator was the");
        println!("    bottleneck, not the target. Raise --max-concurrency or");
        println!("    lower --rate, then re-run.");
    }
    if snap.dropped_results > 0 {
        println!();
        println!("  Results:        {} evicted from the retention ring", snap.dropped_results);
        println!("    (report contains only the most recent samples)");
    }

    if !snap.error_counts.is_empty() {
        println!();
        println!("  Errors:");
        for (err, count) in &snap.error_counts {
            println!("    {}  {}", count, err);
        }
    }

    if !snap.status_codes.is_empty() {
        println!();
        println!("  Status Codes:");
        for (code, count) in &snap.status_codes {
            println!("    {}  {}", code, count);
        }
    }

    println!("{}", "═".repeat(60));
}

fn export_reports(results: &[crate::core::result::ExperimentResult], prefix: &str) -> anyhow::Result<()> {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let base = format!("{}_{}", prefix, ts);

    // CSV
    let csv_path = format!("{}.csv", base);
    crate::export::export_csv(results, &csv_path)?;
    println!("  Exported: {}", csv_path);

    // JSON
    let json_path = format!("{}.json", base);
    crate::export::export_json(results, &json_path)?;
    println!("  Exported: {}", json_path);

    // Summary
    let summary_path = format!("{}_summary.json", base);
    crate::export::export_summary(results, &summary_path)?;
    println!("  Exported: {}", summary_path);

    Ok(())
}
