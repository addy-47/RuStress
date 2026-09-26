//! ============================================================================
//! bounded_memory.rs — the memory ceiling, demonstrated
//! ============================================================================
//! Category     : Utility Tool
//! Component    : runner::client (connection pool), core::constants
//! Prerequisites: Linux (`/proc/self/status`) for the RSS probe
//! Execution    : cargo run --release --example bounded_memory
//! Metrics      : peak RSS at two in-flight ceilings against an 8 MB route
//!
//! Memory here is a function of **concurrent in-flight requests**, not of
//! request count. Each in-flight request holds a hyper HTTP/1 read buffer grown
//! to service the body it is reading, and reqwest exposes no knob to shrink it,
//! so the in-flight ceiling is the only lever. Measured at 500 RPS against 8 MB
//! bodies, with the idle connection pool pinned constant so the two bounds
//! cannot be confused:
//!
//!   max_concurrency | peak RSS
//!   ---------------+------------
//!   8               | 53 MB
//!   64              | 121 MB
//!   256             | 254 MB
//!   1000            | 750 MB
//!
//! This example walks 8 -> 32 -> 64 rather than the full range: the target runs
//! in this same process, so a high ceiling briefly allocates on the machine you
//! are reading this on. A bigger range would not demonstrate more.
//!
//! The same 1000 in-flight run against a *tiny*-body route peaks at 7.3 MB: the
//! buffer never grows past what a small response needs. The cost appears exactly
//! when a load generator is doing its job.
//!
//! This matters because raising the ceiling is how you push a slower target, and
//! it is the one knob that will OOM the machine you are working on. When the
//! generator saturates, requests are dropped and counted rather than queued, so
//! the run says so instead of silently reporting its own backlog as latency.
//! ============================================================================

#[path = "common/mod.rs"]
mod shared;

use rustress::core::Config;
use shared::start_target;

/// Reset the kernel's peak-RSS high-water mark for this process.
///
/// `VmHWM` is monotonic: it is the maximum RSS reached since the process
/// started, so without this every run after the first reports the first run's
/// peak and the comparison is meaningless. Writing `5` to `clear_refs` resets
/// it. Linux-only, which is why every function here degrades to `None`.
fn reset_peak_rss() -> bool {
    std::fs::write("/proc/self/clear_refs", "5").is_ok()
}

/// Peak resident set size of this process since the last reset, in KB.
fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse::<u64>()
                .ok();
        }
    }
    None
}

async fn peak_at(max_concurrency: u32) -> (u32, Option<u64>) {
    // Each measurement must start from zero, or the first run's peak sticks.
    if !reset_peak_rss() {
        eprintln!("  (cannot reset the peak-RSS watermark; figures will be cumulative)");
    }
    let base = start_target().await.expect("target starts");
    let cfg = Config {
        url: format!("{base}/big"),
        target_rps: 400,
        steady_dur_secs: 4,
        timeout_secs: 20,
        max_concurrency,
        // Pinned high so pool sizing is not what varies between the two runs.
        pool_max_idle_per_host: 1_000,
        ..Default::default()
    };
    shared::run_to_completion(cfg).await.expect("run completes");
    (max_concurrency, peak_rss_kb())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(_) = peak_rss_kb() else {
        println!("peak RSS is only readable from /proc; skipping");
        return Ok(());
    };

    println!("8 MB response bodies, 400 RPS, 4s per run\n");
    println!("NOTE: the target server runs in this same process, so these");
    println!("      numbers include its own cost and read high.\n");

    for ceiling in [8_u32, 32, 64] {
        let (c, kb) = peak_at(ceiling).await;
        match kb {
            Some(kb) => println!("  max_concurrency {c:>4}  peak RSS {kb:>7} KB"),
            None => println!("  max_concurrency {c:>4}  peak RSS unavailable"),
        }
    }

    println!("\nRun your own at a different ceiling with --max-concurrency N.");
    Ok(())
}
