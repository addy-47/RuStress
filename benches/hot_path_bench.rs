//! ============================================================================
//! hot_path_bench.rs — per-stage decomposition of the request path
//! ============================================================================
//! Category     : Benchmark
//! Component    : `runner::request::PreparedRequest`, `templates::TemplateEngine`,
//!                `runner::stats::RunStats`, and the wire exchange
//! Prerequisites: none (the crate's own axum target on an OS-assigned port)
//! Execution    : cargo bench --bench hot_path_bench
//! Metrics      : ns/op per stage; allocations per operation with a hard ceiling
//!
//! Sweep without recompiling:
//!   RUSTRESS_BENCH_ITERS=200000 RUSTRESS_BENCH_WIRE_ITERS=5000 \
//!     cargo bench --bench hot_path_bench
//!
//! # Why allocation ceilings
//!
//! A latency threshold is a poor guard for a load generator. A per-request
//! `minijinja::Environment::new()` costs tens of microseconds, which is easily
//! lost in noise on a shared runner, and it allocates two orders of magnitude
//! more than the rest of the path. An allocation count has no such noise. Each
//! stage below therefore asserts a ceiling on allocs/op, and that assertion is
//! the part that fails when the hot path regresses.
//!
//! # Stage boundaries
//!
//! `runner::executor::read_response` is private, so the wire and drain stages
//! are measured through the public `reqwest` surface with a short drain loop.
//! That loop is a measurement probe, not a reimplementation under assertion: no
//! assertion here depends on its behaviour.
//!
//! # Fixture choice
//!
//! The latency stages hit `/sized/64` rather than `/fast`. `/fast` sleeps
//! 10-50 ms, which would make a 2,000-iteration stage take a minute of wall
//! clock and measure the target's jitter rather than the engine. Throughput is
//! measured separately over `/big-ok`, where the size is the point.

use std::alloc::{GlobalAlloc, Layout, System};
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use rustress::core::config::Config;
use rustress::core::result::ExperimentResult;
use rustress::runner::RunStats;
use rustress::runner::client::build_client;
use rustress::runner::request::PreparedRequest;
use rustress::templates::{TemplateContext, TemplateEngine};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Allocation counting
// ---------------------------------------------------------------------------

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

struct Counting;

fn bump() {
    if COUNTING.load(Ordering::Relaxed) {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: every method forwards to `System` unchanged. The counter observes the
// allocation; it does not alter it.
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

/// Run `f` `iters` times; return `(last, allocs_per_op, ns_per_op)`.
///
/// A warm-up pass runs first so first-touch page faults and lazily-built caches
/// are not charged to the measurement.
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

// ---------------------------------------------------------------------------
// Ceilings
//
// Each is derived from the number of allocations the stage is allowed to need,
// not from a measurement. Tightening one is a deliberate act.
// ---------------------------------------------------------------------------

/// `PreparedRequest::build` on a request with no template directive: the URL
/// string, the `RequestBuilder`, and the header map it copies into. A ceiling
/// comfortably above that catches a newly introduced per-request allocation
/// without flagging ordinary `String` growth.
const CEILING_STATIC_BUILD_ALLOCS: f64 = 24.0;

/// `RunStats::record` on a result with no error and no captured body. It takes
/// one status-map lock and pushes into the ring; after the first insertion of a
/// given status code it must not allocate per measurement.
const CEILING_RECORD_ALLOCS: f64 = 2.0;

/// `TemplateEngine::execute_str` builds a minijinja environment and parses the
/// template on every call, so it is allocation-heavy by construction. This
/// ceiling is deliberately loose: it catches an order-of-magnitude regression
/// and nothing finer. Removing the per-call `Environment::new()` would *lower*
/// this number by roughly two orders of magnitude, which no ceiling can reward
/// automatically — see the note printed at the end of the run.
const CEILING_RENDER_ALLOCS: f64 = 4_000.0;

// ---------------------------------------------------------------------------
// Fixtures and configuration
// ---------------------------------------------------------------------------

/// Bind the crate's own reference target on an ephemeral port.
async fn target() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("read bound address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, rustress::dummy::server::DummyServer::router()).await;
    });
    format!("http://{addr}")
}

fn iters_from_env(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A completed 200 with no error and no captured body: the cheapest fold.
fn cheap_result() -> ExperimentResult {
    ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: std::time::Duration::from_micros(1_000),
        service_time: std::time::Duration::from_micros(1_000),
        queue_wait: std::time::Duration::from_micros(1),
        status: 200,
        success: true,
        bytes: 64,
        user_id: "bench".into(),
        query: "custom".into(),
        error: None,
        response_body: None,
    }
}

/// Print one stage row and enforce its allocation ceiling.
fn report(stage: &str, ns: f64, allocs: Option<f64>, ceiling: Option<f64>) {
    let ns_cell = if ns.is_finite() {
        format!("{ns:.1}")
    } else {
        "-".to_string()
    };
    let allocs_cell = allocs
        .filter(|a| a.is_finite())
        .map(|a| format!("{a:.2}"))
        .unwrap_or_else(|| "-".to_string());
    let ceiling_cell = ceiling
        .map(|c| format!("{c:.0}"))
        .unwrap_or_else(|| "-".to_string());
    println!("{stage:<36} {ns_cell:>12} {allocs_cell:>12} {ceiling_cell:>10}");

    if let (Some(ceiling), Some(allocs)) = (ceiling, allocs) {
        assert!(
            allocs.is_finite() && allocs <= ceiling,
            "{stage}: {allocs:.2} allocs/op exceeds the ceiling of {ceiling:.0}.\n\
             A new per-request allocation has appeared on the hot path. A latency\n\
             check would not have caught this; the allocation count does."
        );
    }
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
    if !require_release_build("hot_path_bench") {
        return;
    }

    let cpu_iters = iters_from_env("RUSTRESS_BENCH_ITERS", 50_000);
    let wire_iters = iters_from_env("RUSTRESS_BENCH_WIRE_ITERS", 2_000);
    let bulk_iters = iters_from_env("RUSTRESS_BENCH_BULK_ITERS", 200);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build runtime");

    runtime.block_on(async {
        let base = target().await;
        println!("target              : {base}");
        println!("cpu iterations/stage: {cpu_iters}");
        println!("wire iterations     : {wire_iters}\n");
        println!(
            "{:<36} {:>12} {:>12} {:>10}",
            "stage", "ns/op", "allocs/op", "ceiling"
        );
        println!("{}", "-".repeat(72));

        let engine = TemplateEngine::new();
        let ctx = TemplateContext::new("bench-user".into());
        let (tx, _rx) = mpsc::unbounded_channel();
        let stats = Arc::new(RunStats::new(tx));

        // --- request construction: once per run, amortised ------------------
        let static_cfg = Config {
            url: format!("{base}/sized?bytes=64"),
            steady_dur_secs: 1,
            ..Default::default()
        };
        let (prepared, ctor_allocs, ctor_ns) = measure(cpu_iters / 100, || {
            PreparedRequest::new(&static_cfg, &engine).expect("static request decomposes")
        });
        assert!(
            !prepared.is_templated(),
            "a request with no directive must never take the rendering path"
        );
        report(
            "PreparedRequest::new (static)",
            ctor_ns,
            Some(ctor_allocs),
            None,
        );

        let client = build_client(&static_cfg).expect("client builds");

        // --- request build, no template --------------------------------------
        let (built, build_allocs, build_ns) = measure(cpu_iters, || {
            prepared
                .build(&client, &engine, &ctx)
                .expect("build must succeed")
        });
        std::hint::black_box(&built);
        report(
            "PreparedRequest::build (static)",
            build_ns,
            Some(build_allocs),
            Some(CEILING_STATIC_BUILD_ALLOCS),
        );

        // --- request build, templated URL ------------------------------------
        let templated_cfg = Config {
            url: format!("{base}/sized?bytes=64&u={{{{ user_id }}}}"),
            steady_dur_secs: 1,
            ..Default::default()
        };
        let templated = PreparedRequest::new(&templated_cfg, &engine).expect("templated");
        assert!(
            templated.is_templated(),
            "a URL carrying a directive must take the rendering path"
        );
        let (built, tmpl_allocs, tmpl_ns) = measure(cpu_iters / 10, || {
            templated
                .build(&client, &engine, &ctx)
                .expect("build must succeed")
        });
        std::hint::black_box(&built);
        report(
            "PreparedRequest::build (templated)",
            tmpl_ns,
            Some(tmpl_allocs),
            Some(CEILING_RENDER_ALLOCS),
        );

        // --- template rendering alone ----------------------------------------
        let template = "{{ user_id }}-{{ uuid() }}";
        let (rendered, render_allocs, render_ns) = measure(cpu_iters / 10, || {
            engine.execute_str(template, &ctx).expect("render succeeds")
        });
        std::hint::black_box(&rendered);
        report(
            "TemplateEngine::execute_str",
            render_ns,
            Some(render_allocs),
            Some(CEILING_RENDER_ALLOCS),
        );

        // --- wire: time to response headers, then drain to EOF ---------------
        let mut wire_ns = 0.0f64;
        let mut drain_ns = 0.0f64;
        let mut body_bytes = 0u64;
        for _ in 0..wire_iters {
            let request = prepared
                .build(&client, &engine, &ctx)
                .expect("build must succeed");
            let t_send = Instant::now();
            let mut response = request.send().await.expect("send must succeed");
            let t_wire = Instant::now();
            let mut bytes = 0u64;
            while let Some(chunk) = response.chunk().await.expect("drain succeeds") {
                bytes += chunk.len() as u64;
            }
            let t_drain = Instant::now();
            wire_ns += (t_wire - t_send).as_nanos() as f64;
            drain_ns += (t_drain - t_wire).as_nanos() as f64;
            body_bytes += bytes;
        }
        report(
            "wire: request -> headers",
            wire_ns / wire_iters as f64,
            None,
            None,
        );
        report(
            "wire: drain body to EOF",
            drain_ns / wire_iters as f64,
            None,
            None,
        );

        // --- stats folding ----------------------------------------------------
        let (folded, record_allocs, record_ns) =
            measure(cpu_iters, || stats.record(cheap_result()));
        std::hint::black_box(&folded);
        report(
            "RunStats::record (200, no body)",
            record_ns,
            Some(record_allocs),
            Some(CEILING_RECORD_ALLOCS),
        );
        assert_eq!(
            stats.snapshot().requests,
            (cpu_iters + cpu_iters / 10) as u64,
            "every fold must be counted exactly once; a lost or double-counted \
             measurement shows here as a counter that disagrees with the number \
             of operations actually performed (iterations plus warm-up)"
        );

        // --- throughput over a large body --------------------------------------
        let bulk_cfg = Config {
            url: format!("{base}/big-ok"),
            steady_dur_secs: 1,
            ..Default::default()
        };
        let bulk = PreparedRequest::new(&bulk_cfg, &engine).expect("bulk request");
        let mut bulk_bytes = 0u64;
        let bulk_started = Instant::now();
        for _ in 0..bulk_iters {
            let request = bulk
                .build(&client, &engine, &ctx)
                .expect("build must succeed");
            let mut response = request.send().await.expect("send must succeed");
            while let Some(chunk) = response.chunk().await.expect("drain succeeds") {
                bulk_bytes += chunk.len() as u64;
            }
        }
        let bulk_elapsed = bulk_started.elapsed().as_secs_f64();

        println!("\nsmall-body bytes   : {body_bytes} over {wire_iters} requests");
        println!(
            "large-body transfer: {:.2} MiB in {bulk_elapsed:.3}s = {:.1} MiB/s \
             (sequential, one request at a time)",
            bulk_bytes as f64 / 1024.0 / 1024.0,
            bulk_bytes as f64 / 1024.0 / 1024.0 / bulk_elapsed
        );
        println!(
            "\nNOTE: the templated and render stages are dominated by a per-call\n\
             minijinja Environment::new() inside TemplateEngine::execute_str. That\n\
             contradicts backend-style-guide section 4 ('no template engine\n\
             construction per request') and is an open finding, not an approved cost.\n\
             Removing it would drop those two rows by roughly two orders of magnitude."
        );
    });
}
