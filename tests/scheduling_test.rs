//! ============================================================================
//! scheduling_test — open-loop honesty, saturation, drain, cancellation
//! ============================================================================
//! Category     : Integration Test
//! Component    : `runner::engine` (schedule, shed, drain) + `runner::stats`
//! Prerequisites: none (real axum targets on OS-assigned ephemeral ports)
//! Execution    : cargo test --test scheduling_test
//! Metrics      : requests executed, requests shed, in-flight high-water mark,
//!                retained results, wall-clock duration
//! ============================================================================
//!
//! A load generator that reports a rate it did not achieve is worse than one
//! that reports nothing. These tests pin the four claims the engine makes about
//! its own behaviour: it executes what it scheduled, it says so when it cannot,
//! it finishes the work it counted, and it stops when told to.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;
use tokio::time::sleep;

use common::{Harness, TestServer, cfg_for, serve};

/// Latency of the deliberately slow fixture. Bounded so a drain can never be
/// unbounded; the crate's own `/slow` is 1-2 s of jitter, which would make this
/// file's wall time unpredictable.
const STALL_MS: u64 = 300;

async fn handler_stalled() -> &'static str {
    sleep(Duration::from_millis(STALL_MS)).await;
    "ok"
}

async fn handler_fast() -> &'static str {
    "ok"
}

/// A real target with one instant route and one that always stalls `STALL_MS`.
async fn latency_server() -> TestServer {
    serve(
        Router::new()
            .route("/fast", get(handler_fast))
            .route("/stalled", get(handler_stalled)),
    )
    .await
}

/// Sample `stats.inflight_count()` on a tight cadence; resolves to the maximum.
///
/// Returns the handle rather than awaiting, so the caller can run the engine
/// and only then release the sampler.
fn start_inflight_sampler(
    stats: Arc<rustress::runner::RunStats>,
    stop: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<i64> {
    let peak = Arc::new(AtomicI64::new(0));
    tokio::spawn(async move {
        while !stop.load(Ordering::Relaxed) {
            peak.fetch_max(stats.inflight_count(), Ordering::Relaxed);
            sleep(Duration::from_millis(2)).await;
        }
        peak.load(Ordering::Relaxed)
    })
}

/// The open loop executes the rate it was asked for, within a stated tolerance.
///
/// Catches a scheduler that drifts, dispatches on a fixed sleep instead of a
/// timeline, or silently sheds while reporting a clean run. Tolerance is +/-15%
/// of `target_rps x steady_dur_secs`, which absorbs timer jitter and a debug
/// build while still failing any real rate error.
#[tokio::test]
async fn open_loop_executes_the_requested_rate() {
    const RPS: u32 = 100;
    const SECONDS: u64 = 2;
    const TOLERANCE: f64 = 0.15;

    let server = latency_server().await;
    let mut cfg = cfg_for(&server.url("/fast"));
    cfg.target_rps = RPS;
    cfg.steady_dur_secs = SECONDS;
    cfg.max_concurrency = 64;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    harness.run().await;
    let snapshot = harness.snapshot();

    let expected = RPS as f64 * SECONDS as f64;
    let executed = snapshot.requests as f64;
    assert!(
        (executed - expected).abs() <= expected * TOLERANCE,
        "executed {executed} requests, expected about {expected} (+/-{:.0}%)",
        TOLERANCE * 100.0
    );
    assert_eq!(
        snapshot.dropped_scheduled, 0,
        "a target that answers instantly must never saturate the ceiling; a \
         non-zero drop count here means the rate is being met by shedding"
    );
    assert_eq!(
        snapshot.inflight, 0,
        "run() must not return until every admitted request has finished"
    );
    assert_eq!(
        snapshot.dropped_results, 0,
        "this run is far shorter than the retention capacity"
    );
    assert_eq!(
        harness.results().len() as u64,
        snapshot.requests,
        "every executed request must leave a retained result"
    );
}

/// A saturated generator sheds, says so, and never exceeds its ceiling.
///
/// Catches the three ways open-loop honesty breaks: queueing instead of
/// dropping (which converts a generator limit into apparent target latency),
/// dropping without counting, and letting the in-flight count run past the
/// configured ceiling because the permit is acquired inside the spawned task.
#[tokio::test]
async fn a_saturated_generator_sheds_and_holds_the_concurrency_ceiling() {
    const RPS: u32 = 200;
    const CEILING: u32 = 2;
    const SECONDS: u64 = 1;

    let server = latency_server().await;
    let mut cfg = cfg_for(&server.url("/stalled"));
    cfg.target_rps = RPS;
    cfg.steady_dur_secs = SECONDS;
    cfg.max_concurrency = CEILING;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    let stop = Arc::new(AtomicBool::new(false));
    let sampler = start_inflight_sampler(Arc::clone(harness.stats()), Arc::clone(&stop));

    harness.run().await;
    stop.store(true, Ordering::Relaxed);
    let observed_peak = sampler.await.expect("sampler must not panic");

    let snapshot = harness.snapshot();
    assert!(
        snapshot.dropped_scheduled > 0,
        "two permits against a {STALL_MS} ms target at {RPS} rps must saturate; \
         a zero drop count means requests are being queued instead, which \
         reports a generator limit as target latency"
    );
    assert!(
        snapshot.requests < snapshot.dropped_scheduled,
        "the generator, not the target, was the bottleneck: {} executed vs {} shed",
        snapshot.requests,
        snapshot.dropped_scheduled
    );
    assert!(
        snapshot.requests <= 20,
        "two permits at one request per {STALL_MS} ms cannot execute {} requests in \
         {SECONDS}s",
        snapshot.requests
    );
    assert_eq!(
        observed_peak, CEILING as i64,
        "the ceiling must be reached (proving the sampler can see in-flight work) \
         and never exceeded (proving the permit bounds it). Observed high-water \
         mark: {observed_peak}, ceiling: {CEILING}"
    );
    assert_eq!(
        snapshot.inflight, 0,
        "run() must drain every admitted request before returning"
    );
    assert_eq!(
        harness.results().len() as u64,
        snapshot.requests,
        "a drained run must retain one result per executed request"
    );
}

/// The shed counter is not simply always non-zero.
///
/// Catches a drop counter that increments unconditionally, or a ceiling check
/// that sheds even when there is spare capacity — either would make the
/// measurement-validity flag permanently false and train operators to ignore it.
#[tokio::test]
async fn spare_concurrency_sheds_nothing_on_the_same_slow_target() {
    const RPS: u32 = 10;
    const CEILING: u32 = 64;
    const SECONDS: u64 = 1;

    let server = latency_server().await;
    let mut cfg = cfg_for(&server.url("/stalled"));
    cfg.target_rps = RPS;
    cfg.steady_dur_secs = SECONDS;
    cfg.max_concurrency = CEILING;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    harness.run().await;
    let snapshot = harness.snapshot();

    assert!(
        snapshot.requests > 0,
        "the run must have executed requests, otherwise the negative control is vacuous"
    );
    assert_eq!(
        snapshot.dropped_scheduled, 0,
        "64 permits against a {STALL_MS} ms target at {RPS} rps has ample headroom; \
         shedding here would mean the drop path fires without saturation"
    );
    assert_eq!(snapshot.inflight, 0, "run() must drain before returning");
}

/// Cancelling mid-run stops scheduling, drains, and leaves nothing running.
///
/// Catches an engine that abandons admitted work on cancellation (the drain
/// barrier is bypassed), that keeps dispatching after the token fires, or that
/// deadlocks the drain barrier and hangs the process.
#[tokio::test]
async fn cancellation_stops_scheduling_and_drains_cleanly() {
    const RPS: u32 = 100;
    /// Long enough that an uncancelled run would clearly not finish in time.
    const WOULD_RUN_SECONDS: u64 = 30;
    const CANCEL_AFTER: Duration = Duration::from_millis(300);
    const DEADLINE: Duration = Duration::from_secs(10);

    let server = latency_server().await;
    let mut cfg = cfg_for(&server.url("/fast"));
    cfg.target_rps = RPS;
    cfg.steady_dur_secs = WOULD_RUN_SECONDS;
    cfg.max_concurrency = 64;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    let started = Instant::now();
    tokio::time::timeout(DEADLINE, harness.run_cancelling_after(CANCEL_AFTER))
        .await
        .expect("a cancelled run must return; a hang here is a wedged drain barrier");
    let elapsed = started.elapsed();

    let snapshot = harness.snapshot();
    assert!(snapshot.requests > 0, "the run must have done some work");
    assert!(
        elapsed < Duration::from_secs(5),
        "cancellation must take effect promptly, took {elapsed:?}"
    );
    assert!(
        snapshot.requests < RPS as u64 * 5,
        "scheduling must stop on cancel: {} requests is more than a cancelled \
         {}ms run can account for",
        snapshot.requests,
        CANCEL_AFTER.as_millis()
    );
    assert_eq!(
        snapshot.inflight, 0,
        "cancellation must not abandon counted work: in-flight must return to zero"
    );
    assert_eq!(
        harness.results().len() as u64,
        snapshot.requests,
        "every request counted before cancellation must have completed and been retained"
    );

    let settled = harness.snapshot().requests;
    sleep(Duration::from_millis(250)).await;
    assert_eq!(
        harness.snapshot().requests,
        settled,
        "no request may still be in flight after run() returns"
    );
}

/// Cancelling before the first dispatch still returns cleanly.
///
/// Catches a cancellation check placed only after the dispatch loop, which
/// would run a full schedule before noticing the token.
#[tokio::test]
async fn cancellation_before_the_first_tick_terminates_immediately() {
    let server = latency_server().await;
    let mut cfg = cfg_for(&server.url("/fast"));
    cfg.target_rps = 100;
    cfg.steady_dur_secs = 30;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    let cancel = harness.cancel_token();
    cancel.cancel();

    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(5), harness.run())
        .await
        .expect("an already-cancelled run must return promptly");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a pre-cancelled run must not execute a schedule, took {:?}",
        started.elapsed()
    );
    assert_eq!(harness.snapshot().inflight, 0);
    assert_eq!(
        harness.snapshot().dropped_scheduled,
        0,
        "nothing was scheduled, so nothing can have been shed"
    );
}
