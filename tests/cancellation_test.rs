//! ============================================================================
//! cancellation_test.rs — teardown cannot be held hostage by a request
//! ============================================================================
//! Category     : Integration Test
//! Component    : `runner::executor`, `runner::engine::LoadEngine::await_drain`
//! Prerequisites: none (real `sh` child and a real axum target on a socket)
//! Execution    : cargo test --test cancellation_test
//! Metrics      : run wall-clock, failed request count, error key for the abort
//! ============================================================================
//!
//! The TUI awaits the drain barrier while the terminal is still in raw mode.
//! A request that can run forever therefore costs the user their shell. These
//! tests produce genuinely hanging work — a real `sleep 30` child and a real
//! handler that never answers — and assert that both the timeout and explicit
//! cancellation end the run and count the aborted work.

mod common;

use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;

use rustress::core::config::Config;

use common::{Harness, cfg_for, serve};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hanging_script_is_killed_by_the_request_timeout() {
    let cfg = Config {
        command: Some("sleep 30".to_string()),
        target_rps: 2,
        steady_dur_secs: 1,
        timeout_secs: 1,
        ..Default::default()
    };
    let harness = Harness::new(cfg).expect("a script run needs no target url");

    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(12), harness.run())
        .await
        .expect("a timed-out script must not hold the drain barrier");

    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(12),
        "the run took {elapsed:?}; a hung child kept the terminal alive"
    );

    let snap = harness.snapshot();
    assert!(snap.requests > 0, "the script must have been attempted");
    assert_eq!(
        snap.fail, snap.requests,
        "a timed-out script is a failure, not a success: {snap:?}"
    );
    assert!(
        snap.error_counts.keys().any(|k| k.contains("timed out")),
        "the abort must be diagnosable: {:?}",
        snap.error_counts
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_request_in_flight_ends_the_run_promptly() {
    let server = serve(Router::new().route(
        "/forever",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            "ok"
        }),
    ))
    .await;
    let harness = Harness::new(cfg_for(&server.url("/forever"))).expect("engine");

    let started = Instant::now();
    tokio::time::timeout(
        Duration::from_secs(8),
        harness.run_cancelling_after(Duration::from_millis(300)),
    )
    .await
    .expect("cancellation must stop an in-flight request");

    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(8),
        "cancellation took {elapsed:?}; the run waited for the target's timeout"
    );

    let snap = harness.snapshot();
    assert!(
        snap.fail > 0,
        "the cancelled in-flight request must still be counted: {snap:?}"
    );
    assert!(
        snap.error_counts.keys().any(|k| k.contains("cancelled")),
        "a cancelled request must be labelled: {:?}",
        snap.error_counts
    );
}
