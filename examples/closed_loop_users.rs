//! ============================================================================
//! closed_loop_users.rs — N virtual users, each looping
//! ============================================================================
//! Category     : Utility Tool
//! Component    : core::Config, runner::engine::run_users
//! Prerequisites: none (starts the built-in target itself)
//! Execution    : cargo run --release --example closed_loop_users
//! Metrics      : throughput, per-user think time, service-time percentiles
//!
//! Closed loop is bounded by `num_users` and nothing else. Each user sends a
//! request, waits out `think_time_ms`, and repeats — so concurrency rises and
//! falls with how fast the target responds. That is the point: it models a
//! population of users rather than a request rate.
//!
//! Note that `max_concurrency` is *not* a second control here. It is documented
//! as an open-loop bound, and Users mode deliberately has exactly one
//! concurrency control. Setting it does nothing in this mode.
//! ============================================================================

#[path = "common/mod.rs"]
mod shared;

use rustress::core::Config;
use rustress::core::config::Mode;
use shared::{report, run_to_completion, start_target};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = start_target().await?;
    println!("target: {base}/medium");

    let cfg = Config {
        url: format!("{base}/medium"),
        mode: Mode::Users,
        num_users: 25,
        think_time_ms: 200,
        steady_dur_secs: 5,
        timeout_secs: 10,
        ..Default::default()
    };

    report(
        "closed loop, 25 users, 200ms think time",
        &run_to_completion(cfg).await?,
    );
    Ok(())
}
