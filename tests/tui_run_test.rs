//! ============================================================================
//! tui_run_test.rs — the interactive dashboard actually generates load
//! ============================================================================
//! Category     : Integration Test
//! Component    : runner::RunController (the TUI's run lifecycle)
//! Prerequisites: none (real DummyServer on an ephemeral port)
//! Execution    : cargo test --test tui_run_test
//! Metrics      : requests executed, in-flight high-water mark, drain completion
//!
//! WHY THIS EXISTS
//!
//! The interactive TUI constructed a `LoadEngine` and never called `run` on it.
//! `Ctrl+R` flipped the dashboard to "RUNNING" and the counters sat at zero
//! forever, so the product's headline feature was a simulation. Nothing caught
//! it: the TUI needs a TTY, so there was no test.
//!
//! The fix is to keep orchestration out of the event loop. `RunController`
//! owns build → spawn → cancel → drain → join and knows nothing about frames or
//! key events, which makes it assertable here against a real server. The test
//! does not simulate a terminal; it tests the thing the terminal drives.
//!
//! This test fails if the wiring is removed: with no engine started, `requests`
//! stays at zero and the first assertion fails.
//! ============================================================================

use std::time::Duration;

use rustress::core::config::{Config, Mode};
use rustress::runner::RunController;

mod common;
use common::dummy_server;

/// Poll the controller until `predicate` holds or the deadline passes.
async fn wait_for(
    controller: &mut RunController,
    label: &str,
    predicate: impl Fn(&rustress::StatsSnapshot) -> bool,
) -> rustress::StatsSnapshot {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut last = rustress::StatsSnapshot::default();

    while std::time::Instant::now() < deadline {
        if let Some(snap) = controller.poll() {
            if predicate(&snap) {
                return snap;
            }
            last = snap;
        }
        if let Some(stats) = controller.stats() {
            last = stats.snapshot();
            if predicate(&last) {
                return last;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!(
        "timed out waiting for {label}: requests={} success={} fail={} errors={:?}",
        last.requests, last.success, last.fail, last.error_counts
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn starting_a_run_generates_real_traffic() {
    let server = dummy_server().await;
    let cfg = Config {
        url: server.url("/fast"),
        mode: Mode::Rps,
        target_rps: 200,
        steady_dur_secs: 3,
        timeout_secs: 10,
        ..Config::default()
    };

    let mut controller = RunController::new();
    controller.start(cfg).expect("run starts");

    assert!(
        controller.is_running(),
        "a started run must report itself in flight"
    );

    let snap = wait_for(&mut controller, "requests to accumulate", |s| {
        s.requests > 0 && s.success > 0
    })
    .await;

    assert!(
        snap.requests > 0,
        "the dashboard's whole purpose is traffic; zero requests means the \\
         engine was never started"
    );
    assert_eq!(
        snap.success, snap.requests,
        "every request to a healthy route must succeed, got {} of {}",
        snap.success, snap.requests
    );
    assert_eq!(
        snap.fail, 0,
        "no failures expected against the dummy server"
    );

    controller.stop();
    controller.shutdown().await;

    assert!(
        !controller.is_running(),
        "shutdown must leave nothing in flight"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_run_drains_and_keeps_its_counters() {
    let server = dummy_server().await;
    let cfg = Config {
        url: server.url("/fast"),
        mode: Mode::Rps,
        target_rps: 100,
        // Long enough that only the explicit stop can end it, so this exercises
        // cancellation rather than the natural end of the schedule.
        steady_dur_secs: 600,
        timeout_secs: 10,
        ..Config::default()
    };

    let mut controller = RunController::new();
    controller.start(cfg).expect("run starts");

    let before = wait_for(&mut controller, "traffic to start", |s| s.requests > 0).await;

    controller.stop();
    controller.shutdown().await;

    let after = controller
        .stats()
        .expect("counters survive shutdown so the summary can report them")
        .snapshot();

    assert!(
        after.requests >= before.requests,
        "counters must not go backwards across a stop: {} -> {}",
        before.requests,
        after.requests
    );
    assert!(
        after.inflight == 0,
        "the drain barrier must release every in-flight request, got {}",
        after.inflight
    );
    assert!(
        !controller.is_running(),
        "the run task must be joined, not abandoned"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_start_is_refused_rather_than_silently_replacing_the_run() {
    let server = dummy_server().await;
    let cfg = Config {
        url: server.url("/fast"),
        steady_dur_secs: 600,
        ..Config::default()
    };

    let mut controller = RunController::new();
    controller.start(cfg.clone()).expect("first run starts");

    let second = controller.start(cfg);
    assert!(
        second.is_err(),
        "starting a second run while one is in flight must be refused, not \
         silently orphan the first run's task and its counters"
    );

    controller.stop();
    controller.shutdown().await;
}
