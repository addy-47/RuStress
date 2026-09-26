//! ============================================================================
//! minimal_rps.rs — the smallest useful open-loop load test
//! ============================================================================
//! Category     : Utility Tool
//! Component    : core::Config, runner::LoadEngine
//! Prerequisites: none (starts the built-in target itself)
//! Execution    : cargo run --release --example minimal_rps
//! Metrics      : requests, success, failure, service-time percentiles
//!
//! Open loop (RPS) keeps wall-clock time: requests are scheduled against a
//! timeline, and if the target cannot keep up the generator *drops and counts*
//! them rather than queueing. Queueing would convert the generator's own limit
//! into apparent target latency — coordinated omission — so a non-zero
//! `dropped_scheduled` means the latency numbers describe this machine, not the
//! server under test.
//! ============================================================================

#[path = "common/mod.rs"]
mod shared;

use rustress::core::Config;
use shared::{report, run_to_completion, start_target};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = start_target().await?;
    println!("target: {base}/fast");

    let cfg = Config {
        url: format!("{base}/fast"),
        target_rps: 500,
        steady_dur_secs: 5,
        timeout_secs: 10,
        ..Default::default()
    };

    report("open loop, 500 RPS for 5s", &run_to_completion(cfg).await?);
    Ok(())
}
