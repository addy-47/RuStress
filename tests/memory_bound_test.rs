//! ============================================================================
//! memory_bound_test — the OOM regression test
//! ============================================================================
//! Category     : Integration Test
//! Component    : `runner::executor` + `runner::result_log` resident footprint
//! Prerequisites: Linux (`/proc/self/status` is the only portable-free source
//!                of resident-set size); real axum target on an ephemeral port
//! Execution    : cargo test --test memory_bound_test
//! Metrics      : peak `VmRSS` growth, requests executed, requests retained
//! ============================================================================
//!
//! The tool OOM'd its author's machine by retaining every response body. This
//! file is the guard. It is deliberately built as a *scaling* test rather than
//! an absolute one: two runs against the same 8 MB-error target, identical
//! concurrency, different request counts. Resident memory must not grow with
//! the request count. An absolute ceiling would be dominated by the target
//! server's own footprint — which shares this process — and by allocator
//! noise, neither of which is the property under test.
//!
//! # Process-level constraint
//!
//! Resident memory is a process-wide quantity, so these two tests must not run
//! concurrently with anything else in the same binary. `cargo test` runs test
//! *binaries* sequentially, and `RSS_PROBE_LOCK` serialises the tests within
//! this one. A runner that parallelises across binaries (for example
//! `cargo nextest`) will break this file; that is a property of RSS, not of the
//! code under test.

#![cfg(target_os = "linux")]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rustress::core::config::Config;
use rustress::core::result::ExperimentResult;
use rustress::core::snapshot::StatsSnapshot;
use rustress::dummy::server::BIG_BODY_BYTES;
use rustress::runner::result_log::ResultLog;
use tokio::time::Duration;

use common::{Harness, TestServer, cfg_for, dummy_server, rss_bytes};

/// Serialises the resident-memory tests inside this binary.
///
/// Async-aware because the tests are: a `std::sync::MutexGuard` held across an
/// await is both a clippy error and a `!Send` value that would tie the test to
/// a single runtime thread.
static RSS_PROBE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Concurrency held constant across both measured runs.
///
/// The target shares this process, so its own per-request 8 MB allocation
/// scales with *concurrency*. Holding concurrency fixed removes the target's
/// footprint from the delta and leaves only the request-count term.
const CONCURRENCY: u32 = 8;

const RPS: u32 = 50;

/// Requests that may be added between the two runs for the comparison to mean
/// anything. Below this the delta is inside allocator noise and the test would
/// pass vacuously.
const MIN_DELTA_REQUESTS: u64 = 50;

/// Ceiling on how much extra resident memory 100 extra 8 MB error responses may
/// cost.
///
/// Retaining every body — the original bug — costs `100 x 8 MB = 800 MB` for
/// this delta. A correct implementation retains `100 x 2 KB = 200 KB`. The
/// threshold sits 12x above the correct figure and 12x below the bug, so
/// neither a regression nor allocator noise can flip it.
const MAX_GROWTH_PER_DELTA: u64 = 64 * 1024 * 1024;

/// Build a run config against `server`, sized by wall-clock seconds.
fn big_body_config(server: &TestServer, seconds: u64) -> Config {
    let mut cfg = cfg_for(&server.url("/big"));
    cfg.target_rps = RPS;
    cfg.steady_dur_secs = seconds;
    cfg.max_concurrency = CONCURRENCY;
    cfg.timeout_secs = 10;
    cfg
}

/// A completed 8 MB body, as a target-controlled error page would produce.
fn retained_big_error() -> ExperimentResult {
    ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: Duration::from_millis(1),
        service_time: Duration::from_millis(1),
        queue_wait: Duration::ZERO,
        status: 500,
        success: false,
        bytes: BIG_BODY_BYTES as i64,
        user_id: "u".into(),
        query: "custom".into(),
        error: Some("internal".into()),
        response_body: Some("x".repeat(BIG_BODY_BYTES)),
    }
}

/// Sample resident memory while `run` executes and report the peak growth.
async fn peak_growth_during(server: &TestServer, seconds: u64) -> (u64, StatsSnapshot) {
    let harness = Harness::new(big_body_config(server, seconds)).expect("engine should build");

    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicU64::new(0));
    let sampler = tokio::spawn({
        let stop = Arc::clone(&stop);
        let peak = Arc::clone(&peak);
        async move {
            let baseline = rss_bytes().expect("linux exposes VmRSS");
            while !stop.load(Ordering::Relaxed) {
                if let Some(current) = rss_bytes() {
                    peak.fetch_max(current, Ordering::Relaxed);
                }
                tokio::time::sleep(Duration::from_millis(4)).await;
            }
            peak.load(Ordering::Relaxed).saturating_sub(baseline)
        }
    });

    harness.run().await;
    stop.store(true, Ordering::Relaxed);
    (
        sampler.await.expect("sampler must not panic"),
        harness.snapshot(),
    )
}

/// Two runs at identical concurrency and different request counts must cost
/// the same resident memory.
///
/// Catches the original OOM: an unbounded `Vec<ExperimentResult>` (or an
/// uncapped `response_body`) makes the second run cost
/// `delta x 8 MB` more than the first. A bounded implementation makes the
/// second run cost `delta x 2 KB` more, which is noise.
#[ignore = "allocates hundreds of MB against an 8 MB route; this host has OOM-killed a desktop session. Run explicitly: cargo test --test memory_bound_test -- --ignored"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resident_memory_does_not_scale_with_request_count() {
    let _guard = RSS_PROBE_LOCK.lock().await;
    let server = dummy_server().await;

    let (short_growth, short_snap) = peak_growth_during(&server, 1).await;
    let (long_growth, long_snap) = peak_growth_during(&server, 3).await;

    let delta_requests = long_snap.requests.saturating_sub(short_snap.requests);
    assert!(
        delta_requests >= MIN_DELTA_REQUESTS,
        "the long run must execute at least {MIN_DELTA_REQUESTS} more requests than the \
         short one, or the comparison is vacuous (short={}, long={})",
        short_snap.requests,
        long_snap.requests
    );

    let growth = long_growth.saturating_sub(short_growth);
    let unbounded_cost = delta_requests * BIG_BODY_BYTES as u64;
    println!(
        "short run: {} requests, peak growth {} KiB",
        short_snap.requests,
        short_growth / 1024
    );
    println!(
        "long  run: {} requests, peak growth {} KiB",
        long_snap.requests,
        long_growth / 1024
    );
    println!(
        "delta   : {delta_requests} extra 8 MB error responses cost {} KiB \
         (ceiling {} KiB; retaining every body would cost {} MiB)",
        growth / 1024,
        MAX_GROWTH_PER_DELTA / 1024,
        unbounded_cost / 1024 / 1024
    );
    assert!(
        growth < MAX_GROWTH_PER_DELTA,
        "resident memory grew by {growth} bytes for {delta_requests} extra 8 MB error \
         responses. Retaining every body would cost {unbounded_cost} bytes; a bounded \
         capture costs {}. (short run peak growth {short_growth}, long run {long_growth})",
        delta_requests * rustress::core::constants::MAX_CAPTURED_BODY_BYTES as u64
    );
}

/// The probe must be able to see retention of large bodies, or the test above
/// certifies nothing.
///
/// This is the control for [`resident_memory_does_not_scale_with_request_count`].
/// It deliberately retains bodies the way the bug did and proves the RSS reading
/// moves. Without it, a probe stuck at zero would make the bound above pass
/// unconditionally.
#[ignore = "allocates hundreds of MB against an 8 MB route; this host has OOM-killed a desktop session. Run explicitly: cargo test --test memory_bound_test -- --ignored"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_resident_memory_probe_detects_retained_bodies() {
    let _guard = RSS_PROBE_LOCK.lock().await;

    const RETAINED: usize = 30;
    let floor = RETAINED as u64 * BIG_BODY_BYTES as u64 / 2;

    let before = rss_bytes().expect("linux exposes VmRSS");
    let log = ResultLog::new(RETAINED);
    for _ in 0..RETAINED {
        log.push(retained_big_error());
    }
    let peak = rss_bytes().expect("linux exposes VmRSS");

    assert_eq!(
        log.len(),
        RETAINED,
        "the control must actually retain the bodies"
    );
    common::assert_grew_by_at_least("retained-bodies control", before, peak, floor);

    drop(log);
}
