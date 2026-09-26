use crate::core::config::Config;
use crate::core::snapshot::StatsSnapshot;
use crate::runner::LoadEngine;
use tokio::sync::mpsc;

/// Run the interactive TUI.
pub async fn run_tui(cfg: Config) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::unbounded_channel();
    let engine = LoadEngine::new(cfg.clone(), tx)?;

    crate::tui::run_tui(cfg, rx).await?;

    // Report from the authoritative counters. The retention ring holds at most
    // RESULT_RING_CAPACITY samples, so deriving totals from it would print
    // "Total: 50000" for a three-million-request run and compute the success
    // rate over the last 50k alone.
    let final_snap = engine.stats().snapshot();
    if final_snap.requests > 0 {
        print_summary(&final_snap);
    }

    Ok(())
}

/// Print a post-run summary from a stats snapshot.
fn print_summary(snap: &StatsSnapshot) {
    let total = snap.requests;

    println!("\n{}", "═".repeat(60));
    println!("  SUMMARY");
    println!("{}", "═".repeat(60));
    println!("  Total:     {}", total);
    println!("  Success:   {}", snap.success);
    println!("  Failed:    {}", snap.fail);
    if total > 0 {
        println!(
            "  Rate:      {:.1}%",
            snap.success as f64 / total as f64 * 100.0
        );
    }
    println!("  P50:       {:.1}ms", snap.p50_service_ms);
    println!("  P90:       {:.1}ms", snap.p90_service_ms);
    println!("  P95:       {:.1}ms", snap.p95_service_ms);
    println!("  P99:       {:.1}ms", snap.p99_service_ms);
    println!("  Max:       {:.1}ms", snap.max_service_ms);
    println!("  Bytes:     {}", snap.bytes);

    if snap.dropped_scheduled > 0 {
        println!();
        println!(
            "  ⚠ {} REQUEST(S) SHED — the generator was saturated, so the",
            snap.dropped_scheduled
        );
        println!("    latency figures above do not describe the target.");
    }
    if snap.dropped_results > 0 {
        println!();
        println!(
            "  {} result(s) were evicted from the retention ring and are",
            snap.dropped_results
        );
        println!("    absent from any exported report.");
    }

    println!("{}", "═".repeat(60));
}
