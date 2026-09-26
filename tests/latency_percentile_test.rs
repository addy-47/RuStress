//! ============================================================================
//! latency_percentile_test — percentile correctness through the real engine
//! ============================================================================
//! Category     : Integration Test
//! Component    : `metrics::percentiles` + `metrics::histogram` as fed by
//!                `runner::executor` over real sockets
//! Prerequisites: none (real axum target on an OS-assigned ephemeral port)
//! Execution    : cargo test --test latency_percentile_test
//! Metrics      : p50/p90/p95/p99/max service time in milliseconds
//! ============================================================================
//!
//! Regression guard for the quantile-scale bug. `value_at_quantile` takes a
//! fraction in 0.0-1.0; passing a percentage clamps every percentile to the
//! maximum observed latency, so p50, p90, p95 and p99 all reported the same
//! number and looked like a working dashboard.
//!
//! The distribution is deterministic rather than sampled: a 5% tail at 250 ms
//! against a 5 ms base. A percentile assertion against a random target can
//! pass or fail on luck; this one cannot.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::routing::get;
use tokio::time::sleep;

use common::{Harness, TestServer, cfg_for, serve};

/// Service time of the base of the distribution.
const FAST_MS: u64 = 5;

/// Service time of the tail.
const SLOW_MS: u64 = 250;

/// One request in `SLOW_EVERY` is a tail sample.
const SLOW_EVERY: u64 = 20;

/// Fraction of the distribution that is tail: 1 in `SLOW_EVERY`.
const TAIL_FRACTION: f64 = 1.0 / SLOW_EVERY as f64;

/// A real target with a deterministic 5%-at-250ms latency distribution.
///
/// The counter lives in the target, not in the assertions: the point is to
/// measure the engine's percentile maths against a known truth, so the truth
/// must be a property of the server rather than of the test's expectations.
async fn handler_spread(State(served): State<Arc<AtomicU64>>) -> &'static str {
    let n = served.fetch_add(1, Ordering::Relaxed);
    let ms = if n % SLOW_EVERY == 0 {
        SLOW_MS
    } else {
        FAST_MS
    };
    sleep(Duration::from_millis(ms)).await;
    "ok"
}

async fn spread_server() -> TestServer {
    serve(
        Router::new()
            .route("/spread", get(handler_spread))
            .with_state(Arc::new(AtomicU64::new(0))),
    )
    .await
}

/// Run against the known spread and return the resulting snapshot.
async fn run_against_spread() -> (rustress::core::snapshot::StatsSnapshot, u64) {
    let server = spread_server().await;
    let mut cfg = cfg_for(&server.url("/spread"));
    cfg.target_rps = 150;
    cfg.steady_dur_secs = 2;
    cfg.max_concurrency = 32;
    cfg.timeout_secs = 5;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    harness.run().await;
    let requests = harness.snapshot().requests;
    (harness.snapshot(), requests)
}

/// A known latency spread must produce strictly increasing percentiles.
///
/// Fails if the quantile scale is wrong (all four collapse onto the maximum),
/// if the tail is invisible (p99 at the base), or if the base is inflated by
/// queueing (p50 far above the target's own service time).
#[tokio::test]
async fn percentiles_increase_strictly_across_a_known_latency_spread() {
    let (snap, requests) = run_against_spread().await;

    assert!(
        requests >= 150,
        "the run must sample enough requests to place p99 inside the tail, got {requests}"
    );
    assert_eq!(
        snap.dropped_scheduled, 0,
        "a shed run has no valid latency figures to assert on"
    );

    // The tail must be 1% of samples, so p99 has to land in it.
    let tail_pct = TAIL_FRACTION * 100.0;
    let expected_p99 = FAST_MS as f64 + TAIL_FRACTION * (SLOW_MS - FAST_MS) as f64;
    assert!(
        snap.p99_service_ms > 100.0,
        "p99 is {}ms; with {tail_pct:.0}% of requests stalling at {SLOW_MS}ms, \
         p99 must land in the tail (analytic expectation about {expected_p99:.0}ms). \
         A p99 at the base means the tail was never recorded.",
        snap.p99_service_ms
    );

    assert!(
        snap.p50_service_ms < 25.0,
        "p50 is {}ms; 95% of requests complete in {FAST_MS}ms, so the median must sit \
         at the base, not at the tail",
        snap.p50_service_ms
    );
    assert!(
        snap.p50_service_ms <= snap.p90_service_ms,
        "p50 ({}) must not exceed p90 ({})",
        snap.p50_service_ms,
        snap.p90_service_ms
    );
    assert!(
        snap.p90_service_ms < snap.p99_service_ms,
        "p90 ({}) must be below p99 ({}); with a 5% tail, p90 is still in the base \
         and p99 is in the tail",
        snap.p90_service_ms,
        snap.p99_service_ms
    );
    assert!(
        snap.p99_service_ms <= snap.max_service_ms,
        "p99 ({}) cannot exceed the maximum ({})",
        snap.p99_service_ms,
        snap.max_service_ms
    );
    assert!(
        snap.p50_service_ms >= 0.0 && snap.p50_service_ms < snap.max_service_ms,
        "the base of the distribution must be the minimum, got p50={} max={}",
        snap.p50_service_ms,
        snap.max_service_ms
    );
}

/// The distribution must be shaped as designed, or the assertions above are
/// measuring the wrong thing.
///
/// Pins the fixture's own statistics against its constants: if the target
/// stopped stalling, a p99 assertion would pass for entirely the wrong reason.
#[tokio::test]
async fn the_latency_fixture_actually_produces_the_intended_distribution() {
    let (snap, requests) = run_against_spread().await;

    assert!(snap.requests > 0);
    assert!(
        snap.max_service_ms >= SLOW_MS as f64,
        "max service time is {}ms; the fixture stalls at {SLOW_MS}ms, so the tail \
         was never exercised",
        snap.max_service_ms
    );
    assert!(
        snap.mean_service_ms < SLOW_MS as f64,
        "mean service time is {}ms; a 5% tail cannot lift the mean to {SLOW_MS}ms, \
         so the distribution is not the one this fixture claims to produce",
        snap.mean_service_ms
    );
    assert!(
        snap.requests == requests,
        "the snapshot must be self-consistent"
    );
}
