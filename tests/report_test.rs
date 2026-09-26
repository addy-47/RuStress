//! ============================================================================
//! report_test — reports round-trip through real files
//! ============================================================================
//! Category     : Integration Test
//! Component    : `export::{export_csv, export_json, export_summary}` fed by a
//!                real engine run
//! Prerequisites: none (real `tempfile` directory on disk)
//! Execution    : cargo test --test report_test
//! Metrics      : CSV row count, CSV column values, JSON record count,
//!                summary JSON keys and their values
//! ============================================================================
//!
//! The exports used to be derived from the 50k retention ring rather than from
//! the authoritative counters, so any run longer than the ring emitted a second,
//! different set of numbers with nothing marking the file as partial. These
//! tests read real files back and check they agree with the live counters.

mod common;

use std::collections::HashSet;

use rustress::core::snapshot::StatsSnapshot;
use rustress::export::{export_csv, export_json, export_summary};

use common::{Harness, cfg_for, dummy_server};

/// A report of a completed run must be readable and must agree with the run.
#[tokio::test]
async fn a_completed_run_reports_agree_with_its_live_counters() {
    let server = dummy_server().await;
    let mut cfg = cfg_for(&server.url("/fast"));
    cfg.target_rps = 25;
    cfg.steady_dur_secs = 1;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    harness.run().await;

    let snapshot = harness.snapshot();
    let results = harness.results();
    assert!(
        snapshot.requests > 0,
        "the run must have executed requests, otherwise the report proves nothing"
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let csv_path = dir.path().join("run.csv");
    let json_path = dir.path().join("run.json");
    let summary_path = dir.path().join("run_summary.json");

    export_csv(&results, csv_path.to_str().unwrap()).expect("csv export");
    export_json(&results, json_path.to_str().unwrap()).expect("json export");
    export_summary(results.len(), &snapshot, summary_path.to_str().unwrap())
        .expect("summary export");

    let csv = std::fs::read_to_string(&csv_path).expect("read csv back");
    let mut lines = csv.lines();
    let header = lines.next().expect("csv must have a header");
    assert_eq!(
        header,
        "timeStamp,elapsed,label,responseCode,responseMessage,threadName,dataType,\
         success,failureMessage,bytes,sentBytes,grpThreads,allThreads,URL,Latency,\
         IdleTime,Connect",
        "the JMeter column order is the file's contract with downstream tooling"
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(
        rows.len(),
        results.len(),
        "the CSV must hold one row per retained result"
    );
    for row in &rows {
        assert!(
            row.contains(",200,"),
            "every row must carry the status the target returned: {row}"
        );
    }

    let json_text = std::fs::read_to_string(&json_path).expect("read json back");
    let decoded: Vec<rustress::core::result::ExperimentResult> =
        serde_json::from_str(&json_text).expect("json must deserialize back to results");
    assert_eq!(decoded.len(), results.len());
    let statuses: HashSet<u16> = decoded.iter().map(|r| r.status).collect();
    assert_eq!(statuses, HashSet::from([200]));

    let summary: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&summary_path).expect("read summary back"))
            .expect("summary must be valid JSON");
    assert_eq!(summary["total_requests"], snapshot.requests);
    assert_eq!(summary["total_success"], snapshot.success);
    assert_eq!(summary["total_fail"], snapshot.fail);
    assert_eq!(summary["dropped_scheduled"], 0);
    assert_eq!(summary["measurement_valid"], true);
    assert_eq!(summary["results_retained"], results.len());
    assert_eq!(summary["results_evicted_from_ring"], 0);
    let p50 = summary["p50_us"]
        .as_u64()
        .expect("p50_us must be an integer");
    let p99 = summary["p99_us"]
        .as_u64()
        .expect("p99_us must be an integer");
    assert!(
        p50 > 0,
        "a completed run must report a non-zero p50, got {p50}"
    );
    assert!(
        p50 <= p99,
        "p50 must not exceed p99 in the summary: {p50} vs {p99}"
    );
}

/// The summary must state, not hide, that a run shed requests.
///
/// The measurement-validity rule is that a non-zero `dropped_scheduled` makes
/// the run's latency figures describe the generator rather than the target, and
/// that has to be visible in the artifact, not only in the terminal output.
#[tokio::test]
async fn a_shed_run_is_marked_invalid_in_the_written_summary() {
    let server = dummy_server().await;
    let mut cfg = cfg_for(&server.url("/medium"));
    cfg.target_rps = 200;
    cfg.steady_dur_secs = 1;
    cfg.max_concurrency = 2;
    let harness = Harness::new(cfg).expect("config should construct an engine");

    harness.run().await;

    let snapshot = harness.snapshot();
    assert!(
        snapshot.dropped_scheduled > 0,
        "two permits against the 100-300ms /medium route at 200 rps must saturate"
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("shed_summary.json");
    let results = harness.results();
    export_summary(results.len(), &snapshot, path.to_str().unwrap()).expect("summary export");

    let summary: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read summary back"))
            .expect("summary must be valid JSON");
    assert_eq!(summary["dropped_scheduled"], snapshot.dropped_scheduled);
    assert_eq!(
        summary["measurement_valid"], false,
        "a shed run must not present its latency figures as describing the target"
    );
}

/// A run longer than the ring must export the counters, not the ring.
///
/// The failure this guards: percentiles recomputed from the retained window
/// would disagree with the run and nothing in the file would say which was
/// right. Here the executed count is deliberately far above the retained count.
#[test]
fn an_over_capacity_run_reports_counters_not_the_retained_window() {
    let executed = 3_000_000u64;
    let evicted = executed - 50_000;

    let mut snapshot = StatsSnapshot {
        requests: executed,
        success: executed,
        fail: 0,
        p50_service_ms: 12.5,
        p90_service_ms: 30.0,
        p95_service_ms: 44.0,
        p99_service_ms: 120.0,
        max_service_ms: 900.0,
        ..Default::default()
    };
    snapshot.dropped_results = evicted;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("big_summary.json");
    export_summary(50_000, &snapshot, path.to_str().unwrap()).expect("summary export");

    let summary: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read summary back"))
            .expect("summary must be valid JSON");
    assert_eq!(summary["total_requests"], executed);
    assert_eq!(summary["results_retained"], 50_000);
    assert_eq!(summary["results_evicted_from_ring"], evicted);
    assert_eq!(summary["p50_us"], 12_500);
    assert_eq!(summary["p99_us"], 120_000);
    assert_eq!(summary["max_us"], 900_000);
    assert_eq!(summary["measurement_valid"], true);
}
