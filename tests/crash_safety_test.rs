//! ============================================================================
//! crash_safety_test — P0 bounded-memory regressions
//! ============================================================================
//! Category     : Integration Test
//! Component    : `runner::executor` (body capture), `runner::result_log`
//!                (retention), `metrics::collector` (error key space)
//! Prerequisites: none (real axum target on an OS-assigned ephemeral port)
//! Execution    : cargo test --test crash_safety_test
//! Metrics      : captured body bytes, truncated-body marker presence, bytes
//!                transferred, results retained, results evicted, error keys
//! ============================================================================
//!
//! Every bound asserted here exists because the tool previously retained every
//! response body in an unbounded vector and OOM'd its host. Each test states
//! the defect it catches; none of them can pass if the corresponding bound is
//! removed.

mod common;

use std::sync::Arc;
use std::time::Duration;

use rustress::core::constants::{
    ERROR_KEY_OVERFLOW_LABEL, MAX_CAPTURED_BODY_BYTES, MAX_DRAINED_BODY_BYTES,
    MAX_TRACKED_ERROR_KEYS, STATS_CHANNEL_CAPACITY,
};
use rustress::core::result::ExperimentResult;
use rustress::runner::RunStats;
use rustress::runner::executor::TRUNCATION_MARKER;
use rustress::runner::result_log::ResultLog;
use tokio::sync::mpsc;

use common::{Harness, cfg_for, dummy_server};

/// Build a real `ExperimentResult` carrying `error`, for aggregation assertions.
fn transport_error(message: &str) -> ExperimentResult {
    ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: Duration::from_millis(1),
        service_time: Duration::from_millis(1),
        queue_wait: Duration::ZERO,
        status: 0,
        success: false,
        bytes: 0,
        user_id: "u".into(),
        query: "custom".into(),
        error: Some(message.to_string()),
        response_body: None,
    }
}

/// A 500 whose body is exactly `MAX_CAPTURED_BODY_BYTES` is a *complete*
/// capture.
///
/// Catches the off-by-one that would mark every response truncated. That defect
/// sends an operator hunting for a multi-megabyte error page the target never
/// sent.
#[tokio::test]
async fn body_of_exactly_the_capture_cap_is_complete_not_truncated() {
    let server = dummy_server().await;
    let url = server.url(&format!("/sized?bytes={MAX_CAPTURED_BODY_BYTES}"));
    let harness = Harness::new(cfg_for(&url)).expect("config should construct an engine");

    harness.run().await;

    let results = harness.results();
    assert!(
        !results.is_empty(),
        "the engine executed no requests, so nothing was proven about capture"
    );
    for result in &results {
        let body = result
            .response_body
            .as_deref()
            .expect("a 500 body must be captured for diagnostics");
        assert_eq!(
            body.chars().count(),
            MAX_CAPTURED_BODY_BYTES,
            "a body of exactly the cap must be captured whole"
        );
        assert!(
            !body.ends_with(TRUNCATION_MARKER),
            "a body of exactly the cap is complete and must not be marked truncated"
        );
        assert!(
            body.bytes().all(|b| b == b'x'),
            "the captured prefix must be the bytes the server actually sent"
        );
    }
}

/// One byte past the cap is truncated, and says so.
///
/// Catches a capture that keeps the prefix without marking it: the retained text
/// would then be indistinguishable from a complete short body.
#[tokio::test]
async fn body_one_byte_past_the_capture_cap_is_marked_truncated() {
    let server = dummy_server().await;
    let url = server.url(&format!("/sized?bytes={}", MAX_CAPTURED_BODY_BYTES + 1));
    let harness = Harness::new(cfg_for(&url)).expect("config should construct an engine");

    harness.run().await;

    let results = harness.results();
    assert!(!results.is_empty(), "no requests executed");
    for result in &results {
        let body = result
            .response_body
            .as_deref()
            .expect("a 500 body must be captured for diagnostics");
        assert_eq!(
            body.chars().count(),
            MAX_CAPTURED_BODY_BYTES + TRUNCATION_MARKER.chars().count(),
            "capture must stop at the cap and add only the marker"
        );
        assert!(
            body.ends_with(TRUNCATION_MARKER),
            "an over-cap body must be explicitly marked truncated, got {body:?}"
        );
    }
}

/// Capturing a prefix does not stop the transfer from being measured.
///
/// Catches an executor that abandons the body once the capture cap is reached:
/// bytes would stop accumulating and, worse, the connection would be dropped so
/// the next request paid a fresh handshake inside its measured service time.
#[tokio::test]
async fn an_oversized_body_is_drained_to_eof_and_every_byte_is_counted() {
    const BODY_BYTES: u64 = 100_000;

    let server = dummy_server().await;
    let url = server.url(&format!("/sized?bytes={BODY_BYTES}"));
    let harness = Harness::new(cfg_for(&url)).expect("config should construct an engine");

    harness.run().await;

    let results = harness.results();
    assert!(!results.is_empty(), "no requests executed");
    for result in &results {
        assert_eq!(
            result.bytes as u64, BODY_BYTES,
            "every byte on the wire must be counted, not just the captured prefix"
        );
        let body = result
            .response_body
            .as_deref()
            .expect("a 500 body must be captured for diagnostics");
        assert!(
            body.ends_with(TRUNCATION_MARKER),
            "100 KB is far past the 2 KB cap and must be marked truncated"
        );
    }

    let snapshot = harness.snapshot();
    assert_eq!(
        snapshot.bytes,
        results.len() as u64 * BODY_BYTES,
        "the aggregate byte counter must equal the sum of the per-request counts"
    );
}

/// Per-request retention is a fixed-capacity ring, not a growing vector.
///
/// Catches the original OOM exactly: an unbounded `Vec<ExperimentResult>`.
/// 100k results into capacity 16 must leave 16 and account for the other
/// 99,984 — a bound that discards without reporting misleads the report writer.
#[test]
fn retention_ring_holds_capacity_under_100k_results() {
    let log = ResultLog::new(16);
    for _ in 0..100_000 {
        log.push(transport_error("e"));
    }

    assert_eq!(log.len(), 16, "retention must never exceed its capacity");
    assert_eq!(log.capacity(), 16);
    assert_eq!(
        log.dropped_from_front(),
        99_984,
        "every evicted result must be reported, not silently dropped"
    );
}

/// The error key space is target-controlled and must be capped.
///
/// Catches an `IndexMap<String, u64>` that grows with target-supplied text. A
/// target returning a unique error string per request would otherwise add one
/// map entry per request, forever.
#[test]
fn error_key_space_is_capped_and_the_overflow_bucket_holds_the_remainder() {
    const UNIQUE_ERRORS: usize = 3_200;

    let (tx, _rx) = mpsc::channel(STATS_CHANNEL_CAPACITY);
    let stats = Arc::new(RunStats::new(tx));
    for i in 0..UNIQUE_ERRORS {
        stats.record(transport_error(&format!("unique-error-{i}")));
    }

    let snapshot = stats.snapshot();
    assert_eq!(
        snapshot.error_counts.len(),
        MAX_TRACKED_ERROR_KEYS + 1,
        "the error map must cap at the budget plus one overflow bucket"
    );
    assert_eq!(
        snapshot.error_counts.get(ERROR_KEY_OVERFLOW_LABEL),
        Some(&((UNIQUE_ERRORS - MAX_TRACKED_ERROR_KEYS) as u64)),
        "every error past the budget must be accounted for in the overflow bucket"
    );
    assert_eq!(
        snapshot.fail, UNIQUE_ERRORS as u64,
        "a transport error must be counted as a failure"
    );
    assert!(
        snapshot.status_codes.is_empty(),
        "a transport error has no HTTP status and must not be recorded as one"
    );
}

/// An already-tracked error keeps incrementing its own key after the budget is
/// spent.
///
/// Catches a fold that redirects *every* error to the overflow bucket once the
/// budget is full, which would discard the one diagnostic an operator needs.
#[test]
fn tracked_error_keys_keep_counting_after_the_budget_is_spent() {
    const TRACKED_FIRST: &str = "error-0";

    let (tx, _rx) = mpsc::channel(STATS_CHANNEL_CAPACITY);
    let stats = Arc::new(RunStats::new(tx));

    for i in 0..(MAX_TRACKED_ERROR_KEYS + 10) {
        stats.record(transport_error(&format!("error-{i}")));
    }
    for _ in 0..3 {
        stats.record(transport_error(TRACKED_FIRST));
    }

    let snapshot = stats.snapshot();
    assert_eq!(
        snapshot.error_counts.get(TRACKED_FIRST),
        Some(&4),
        "a key already inside the budget must keep its own counter"
    );
    assert_eq!(snapshot.error_counts.len(), MAX_TRACKED_ERROR_KEYS + 1);
}

/// An over-cap body still leaves the target's status visible.
///
/// Catches a capture path that swallows the status alongside the body, which
/// would let a run report 100% success against a target returning only 500s.
#[tokio::test]
async fn error_status_is_recorded_alongside_the_captured_body() {
    let server = dummy_server().await;
    let url = server.url(&format!("/sized?bytes={}", MAX_CAPTURED_BODY_BYTES * 4));
    let harness = Harness::new(cfg_for(&url)).expect("config should construct an engine");

    harness.run().await;

    let snapshot = harness.snapshot();
    assert!(snapshot.requests > 0, "no requests executed");
    assert_eq!(
        snapshot.status_codes.get(&500),
        Some(&snapshot.requests),
        "the target's status must survive body capture: {:?}",
        snapshot.status_codes
    );
    assert_eq!(snapshot.fail, snapshot.requests);
    assert_eq!(snapshot.success, 0);
    assert!(
        snapshot
            .response_samples
            .get(&500)
            .is_some_and(|s| !s.is_empty()),
        "a 500 sample body must be retained for the report"
    );
}

/// Every dispatched request produces exactly one counted, retained result.
///
/// Catches an executor that returns early on a short read, leaving the run
/// reporting fewer requests than it dispatched and no evidence of which ones.
#[tokio::test]
async fn every_dispatched_request_produces_exactly_one_result() {
    let server = dummy_server().await;
    let mut cfg = cfg_for(&server.url("/sized?bytes=4096"));
    cfg.target_rps = 30;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    harness.run().await;

    let snapshot = harness.snapshot();
    let retained = harness.results();
    assert!(
        snapshot.requests > 0,
        "no requests executed, so the accounting below proves nothing"
    );
    assert_eq!(
        retained.len() as u64,
        snapshot.requests,
        "the retention ring must hold one entry per executed request while the \
         run is shorter than its capacity"
    );
    assert_eq!(
        snapshot.dropped_scheduled, 0,
        "an unsaturated run must shed nothing, so the executed count is the honest count"
    );
    assert!(
        retained.iter().all(|r| r.status == 500),
        "every retained result must carry the 500 the target returned"
    );
    assert!(
        retained.iter().all(|r| !r.success),
        "a 500 is not a success"
    );
}

/// The `/sized` fixture refuses a body larger than the drain cap.
///
/// Without this bound the fixture could be pointed at a gigabyte and become the
/// memory hazard it exists to detect.
#[tokio::test]
async fn an_oversized_fixture_request_is_refused_rather_than_served() {
    let server = dummy_server().await;
    let mut cfg = cfg_for(&server.url(&format!("/sized?bytes={}", MAX_DRAINED_BODY_BYTES + 1)));
    cfg.target_rps = 5;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    harness.run().await;

    let snapshot = harness.snapshot();
    assert!(
        snapshot.requests > 0,
        "the request must still have been dispatched, otherwise this proves nothing"
    );
    assert_eq!(
        snapshot.status_codes.get(&400),
        Some(&snapshot.requests),
        "a fixture size past the drain cap must be refused with 400, not served: {:?}",
        snapshot.status_codes
    );
    assert!(
        snapshot.bytes < snapshot.requests * 1_024,
        "a refused request may transfer its refusal message but nothing close to \
         the {} bytes asked for; {} requests moved {} bytes",
        MAX_DRAINED_BODY_BYTES,
        snapshot.requests,
        snapshot.bytes
    );
}
