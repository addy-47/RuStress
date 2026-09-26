//! ============================================================================
//! aggregation_bench.rs — the metrics and retention path, and report writing
//! ============================================================================
//! Category     : Benchmark
//! Component    : `metrics::StatsCollector`, `runner::result_log::ResultLog`,
//!                `runner::stats::RunStats`, `export::{csv, json, summary}`
//! Prerequisites: none (a real `tempfile` directory for the report writers)
//! Execution    : cargo bench --bench aggregation_bench
//! Metrics      : ns/op per stage; allocations per operation with a hard ceiling
//!
//! Sweep without recompiling:
//!   RUSTRESS_BENCH_FOLDS=2000000 RUSTRESS_BENCH_RING=2000000 \
//!   RUSTRESS_BENCH_EXPORT=50000 cargo bench --bench aggregation_bench
//!
//! # Why this path is benchmarked
//!
//! `StatsCollector::add` runs once per request, on every request, and touches
//! three mutex-guarded maps plus a histogram. The retention ring runs once per
//! request too. Neither shows up in an end-to-end number: a regression there is
//! hidden inside the wire time, which the target dominates. A load generator
//! that cannot fold its own metrics quickly cannot saturate anything.
//!
//! The key-space case is measured separately on purpose. Once
//! `MAX_TRACKED_ERROR_KEYS` is spent, every further unique error folds into one
//! bucket, and that path's allocation cost is not the un-saturated path's cost.
//! A regression that made the saturated path allocate per request would be
//! invisible in the common case.

use std::alloc::{GlobalAlloc, Layout, System};
use std::env;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use rustress::core::constants::{MAX_CAPTURED_BODY_BYTES, MAX_TRACKED_ERROR_KEYS};
use rustress::core::result::ExperimentResult;
use rustress::core::snapshot::StatsSnapshot;
use rustress::export::{export_csv, export_json, export_summary};
use rustress::metrics::StatsCollector;
use rustress::runner::RunStats;
use rustress::runner::result_log::ResultLog;
use tokio::sync::mpsc;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

struct Counting;

fn bump() {
    if COUNTING.load(Ordering::Relaxed) {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: every method forwards to `System` unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A plain success fold. Atomics, one status-map lookup, one histogram write.
/// Once the status key exists it must not allocate at all.
const CEILING_PLAIN_FOLD_ALLOCS: f64 = 1.0;

/// An error fold. `bounded_error_key` clones the key string on every call, so
/// one allocation per fold is the current known cost; the ceiling leaves room
/// for exactly one more appearing.
const CEILING_ERROR_FOLD_ALLOCS: f64 = 2.0;

/// `ResultLog::push` past capacity. Eviction removes one entry before the push,
/// so the deque never needs to grow: a reallocation per request is the shape of
/// the original unbounded-growth bug.
const CEILING_RING_PUSH_ALLOCS: f64 = 1.0;

/// A 32x larger captured body must not cost measurably more to fold.
///
/// The captured body is borrowed, never copied, so fold cost is independent of
/// how large the target's error page is. If this ratio ever grows, retention has
/// stopped being bounded by the capture cap and the OOM is back.
const MAX_BODY_SIZE_COST_RATIO: f64 = 3.0;

fn measure<T, F: FnMut() -> T>(iters: usize, mut f: F) -> (T, f64, f64) {
    let warmup = (iters / 10).max(1);
    for _ in 0..warmup {
        std::hint::black_box(f());
    }

    ALLOCS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    let started = Instant::now();
    let mut last = None;
    for _ in 0..iters {
        last = Some(f());
    }
    let elapsed = started.elapsed();
    COUNTING.store(false, Ordering::Relaxed);

    (
        last.expect("at least one iteration ran"),
        ALLOCS.load(Ordering::Relaxed) as f64 / iters as f64,
        elapsed.as_nanos() as f64 / iters as f64,
    )
}

fn result(status: u16, success: bool, error: Option<&str>) -> ExperimentResult {
    ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: std::time::Duration::from_micros(1_000),
        service_time: std::time::Duration::from_micros(1_000),
        queue_wait: std::time::Duration::from_micros(1),
        status,
        success,
        bytes: 64,
        user_id: "bench".into(),
        query: "custom".into(),
        error: error.map(str::to_string),
        response_body: None,
    }
}

/// Same shape, with a captured body of `body_bytes` on the response.
fn result_with_body(body_bytes: usize) -> ExperimentResult {
    ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: std::time::Duration::from_micros(1_000),
        service_time: std::time::Duration::from_micros(1_000),
        queue_wait: std::time::Duration::from_micros(1),
        status: 500,
        success: false,
        bytes: body_bytes as i64,
        user_id: "bench".into(),
        query: "custom".into(),
        error: None,
        response_body: Some("x".repeat(body_bytes)),
    }
}

fn report(stage: &str, ns: f64, allocs: f64, ceiling: Option<f64>) {
    let ceiling_cell = ceiling
        .map(|c| format!("{c:.0}"))
        .unwrap_or_else(|| "-".to_string());
    println!("{stage:<44} {ns:>12.1} {allocs:>12.2} {ceiling_cell:>10}");
    if let Some(ceiling) = ceiling {
        assert!(
            allocs.is_finite() && allocs <= ceiling,
            "{stage}: {allocs:.2} allocs/op exceeds the ceiling of {ceiling:.0}.\n\
             A per-request allocation has appeared on a path that runs once per request."
        );
    }
}

fn iters_from_env(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Refuse to produce numbers from a debug build.
///
/// A load generator's own performance is the product, and a debug build reports
/// latencies up to 7x worse. A benchmark that printed those numbers would look
/// like evidence and be meaningless, and a release-only allocation-ratio
/// assertion evaluated against them would fail for the wrong reason. Declining
/// is the honest outcome, and it is also what keeps `cargo test --all-targets`
/// correct: that command includes bench targets, and with `harness = false` the
/// benchmark's `main` *is* the test entry point.
fn require_release_build(bench: &str) -> bool {
    if cfg!(debug_assertions) {
        eprintln!(
            "{bench} is release-only. Run `cargo bench --bench {bench}`. \
             Debug-build latencies are up to 7x worse than release and would make a \
             correct implementation look broken."
        );
        return false;
    }
    true
}

fn main() {
    if !require_release_build("aggregation_bench") {
        return;
    }

    let folds = iters_from_env("RUSTRESS_BENCH_FOLDS", 1_000_000);
    let ring_pushes = iters_from_env("RUSTRESS_BENCH_RING", 1_000_000);
    let snapshot_iters = iters_from_env("RUSTRESS_BENCH_SNAPSHOT", 50_000);
    let export_n = iters_from_env("RUSTRESS_BENCH_EXPORT", 10_000);

    println!("folds              : {folds}");
    println!("ring pushes        : {ring_pushes}");
    println!("exported results   : {export_n}\n");
    println!(
        "{:<44} {:>12} {:>12} {:>10}",
        "stage", "ns/op", "allocs/op", "ceiling"
    );
    println!("{}", "-".repeat(80));

    // --- fold a plain success -------------------------------------------------
    let collector = StatsCollector::new();
    let (_, plain_allocs, plain_ns) = measure(folds, || collector.add(&result(200, true, None)));
    report(
        "StatsCollector::add (200)",
        plain_ns,
        plain_allocs,
        Some(CEILING_PLAIN_FOLD_ALLOCS),
    );
    assert_eq!(
        collector.snapshot().requests,
        (folds + folds / 10) as u64,
        "every fold must be counted exactly once, including the warm-up pass"
    );

    // --- fold a repeated error, inside the key budget -------------------------
    let repeated = result(0, false, Some("connection refused"));
    let (_, err_allocs, err_ns) = measure(folds, || collector.add(&repeated));
    report(
        "StatsCollector::add (known error key)",
        err_ns,
        err_allocs,
        Some(CEILING_ERROR_FOLD_ALLOCS),
    );

    // --- fold past the key budget, into the overflow bucket --------------------
    let budgeted = StatsCollector::new();
    for i in 0..MAX_TRACKED_ERROR_KEYS {
        budgeted.add(&result(0, false, Some(&format!("seeded-error-{i}"))));
    }
    let (_, over_allocs, over_ns) = measure(folds, || {
        budgeted.add(&result(0, false, Some("a-fresh-unique-error-every-time")))
    });
    report(
        "StatsCollector::add (key budget spent)",
        over_ns,
        over_allocs,
        Some(CEILING_ERROR_FOLD_ALLOCS),
    );
    assert_eq!(
        budgeted.snapshot().error_counts.len(),
        MAX_TRACKED_ERROR_KEYS + 1,
        "a fresh unique error past the budget must fold into the overflow bucket \
         rather than growing the map"
    );

    // --- fold cost must not scale with the captured body -----------------------
    let small = result_with_body(64);
    let large = result_with_body(MAX_CAPTURED_BODY_BYTES);
    let small_collector = StatsCollector::new();
    let large_collector = StatsCollector::new();
    let (_, _, small_body_ns) = measure(folds, || small_collector.add(&small));
    let (_, large_body_allocs, large_body_ns) = measure(folds, || large_collector.add(&large));
    report(
        "StatsCollector::add (64 B captured body)",
        small_body_ns,
        f64::NAN,
        None,
    );
    report(
        "StatsCollector::add (2 KB captured body)",
        large_body_ns,
        large_body_allocs,
        Some(CEILING_ERROR_FOLD_ALLOCS),
    );
    assert!(
        large_body_ns < small_body_ns * MAX_BODY_SIZE_COST_RATIO,
        "folding a {MAX_CAPTURED_BODY_BYTES}-byte captured body cost {:.1} ns against \
         {:.1} ns for 64 bytes (ratio {:.2}, ceiling {MAX_BODY_SIZE_COST_RATIO}). Fold \
         cost must be independent of body size; if it is not, retention is no longer \
         bounded by the capture cap.",
        large_body_ns,
        small_body_ns,
        large_body_ns / small_body_ns.max(f64::MIN_POSITIVE)
    );

    // --- retention ring, amortised across eviction -----------------------------
    let log = ResultLog::new(1_024);
    for _ in 0..1_024 {
        log.push(result(200, true, None));
    }
    let (_, push_allocs, push_ns) = measure(ring_pushes, || log.push(result(200, true, None)));
    report(
        "ResultLog::push (evicting)",
        push_ns,
        push_allocs,
        Some(CEILING_RING_PUSH_ALLOCS),
    );
    assert_eq!(
        log.len(),
        1_024,
        "the ring must hold exactly its capacity, never more"
    );
    assert_eq!(
        log.dropped_from_front(),
        ring_pushes as u64,
        "every eviction must be counted, so the run can state that its report is a \
         partial view of the traffic it generated"
    );

    // --- the UI read path ------------------------------------------------------
    let (tx, _rx) = mpsc::unbounded_channel();
    let run_stats = RunStats::new(tx);
    for _ in 0..10_000 {
        run_stats.record(result(200, true, None));
    }
    let (snap, snap_allocs, snap_ns) = measure(snapshot_iters, || run_stats.snapshot());
    report(
        "RunStats::snapshot (UI cadence)",
        snap_ns,
        snap_allocs,
        None,
    );
    assert_eq!(
        snap.requests, 10_000,
        "a snapshot must report the live counters, not a stale copy"
    );

    // --- report writing --------------------------------------------------------
    let results: Vec<ExperimentResult> = (0..export_n)
        .map(|i| ExperimentResult {
            user_id: format!("user-{i}"),
            ..result(200, true, None)
        })
        .collect();
    let snap = StatsSnapshot {
        requests: export_n as u64,
        success: export_n as u64,
        p50_service_ms: 1.0,
        p99_service_ms: 2.0,
        ..Default::default()
    };

    let dir = tempfile::tempdir().expect("temp dir");
    let csv_path = dir.path().join("bench.csv");
    let json_path = dir.path().join("bench.json");
    let summary_path = dir.path().join("bench_summary.json");

    let (_, _, csv_ns) = measure(1, || {
        export_csv(&results, csv_path.to_str().expect("utf-8 path")).expect("csv export succeeds")
    });
    report("export_csv (whole file)", csv_ns, f64::NAN, None);

    let (_, _, json_ns) = measure(1, || {
        export_json(&results, json_path.to_str().expect("utf-8 path"))
            .expect("json export succeeds")
    });
    report("export_json (whole file)", json_ns, f64::NAN, None);

    let (_, _, summary_ns) = measure(1, || {
        export_summary(
            results.len(),
            &snap,
            summary_path.to_str().expect("utf-8 path"),
        )
        .expect("summary export succeeds")
    });
    report("export_summary (whole file)", summary_ns, f64::NAN, None);

    let csv_bytes = std::fs::metadata(&csv_path).expect("csv written").len();
    let json_bytes = std::fs::metadata(&json_path).expect("json written").len();
    println!("\ncsv bytes          : {csv_bytes}");
    println!("json bytes         : {json_bytes}");
    println!("csv ns/result      : {:.1}", csv_ns / export_n as f64);
    println!("json ns/result     : {:.1}", json_ns / export_n as f64);
}
