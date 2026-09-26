//! ============================================================================
//! shared.rs — helpers used by the runnable examples
//! ============================================================================
//! Category     : Utility Tool (support module, not an example itself)
//! Component    : example scaffolding
//! Prerequisites: none
//!
//! Not a standalone example. Each `examples/*.rs` binary pulls these in so the
//! interesting part of the example is the load configuration rather than the
//! boilerplate every one of them would otherwise repeat.
//! ============================================================================

#![allow(dead_code)]

use std::net::SocketAddr;

use rustress::core::Config;
use rustress::core::constants::STATS_CHANNEL_CAPACITY;
use rustress::core::snapshot::StatsSnapshot;
use rustress::dummy::DummyServer;
use rustress::runner::LoadEngine;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Start the built-in target on an ephemeral port and return its base URL.
///
/// Port 0 lets the OS pick a free port, so parallel examples and CI runners do
/// not collide. This is why `DummyServer::router()` is public: `run()` takes a
/// port and reports none, which would make port 0 unusable.
pub async fn start_target() -> anyhow::Result<String> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let addr = listener.local_addr()?;
    let router = DummyServer::router();

    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    // The listener is already bound, so the address is live; no readiness poll
    // or sleep is needed and none should be added.
    Ok(format!("http://{addr}"))
}

/// Run `cfg` to completion and return the final counters.
///
/// Awaits `LoadEngine::run` itself. An earlier version polled the counters until
/// they were non-zero and returned, which reported 14 of 2500 scheduled
/// requests and looked like a clean run -- the exact false green the testing
/// standards warn about. Completion is the run task finishing, not the first
/// non-zero counter.
pub async fn run_to_completion(cfg: Config) -> anyhow::Result<StatsSnapshot> {
    let (tx, rx) = mpsc::channel(STATS_CHANNEL_CAPACITY);
    let engine = LoadEngine::new(cfg, tx)?;
    let stats = engine.stats().clone();

    // Progress frames are display hints; drain them so the bounded channel
    // never fills while `run` holds the current thread.
    let mut rx = rx;
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });

    engine.run(CancellationToken::new()).await;

    drain.abort();
    Ok(stats.snapshot())
}

/// Print a counter summary in a fixed shape.
pub fn report(label: &str, snap: &StatsSnapshot) {
    println!("\n{label}");
    println!("  requests   {}", snap.requests);
    println!("  success    {}", snap.success);
    println!("  failed     {}", snap.fail);
    println!("  bytes      {}", snap.bytes);
    println!(
        "  latency    p50 {:.2}ms  p95 {:.2}ms  p99 {:.2}ms  max {:.2}ms",
        snap.p50_service_ms, snap.p95_service_ms, snap.p99_service_ms, snap.max_service_ms
    );
    if snap.dropped_scheduled > 0 {
        println!(
            "  NOTE: {} request(s) were shed because the generator saturated.",
            snap.dropped_scheduled
        );
        println!("        Latency figures above do not describe the target.");
    }
}
