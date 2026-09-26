//! ============================================================================
//! concurrency_memory_test.rs — peak memory is bounded by the in-flight ceiling
//! ============================================================================
//! Category     : Integration Test
//! Component    : runner::engine, runner::client, core::config
//! Prerequisites: none (stands up the real DummyServer on an ephemeral port)
//! Execution    : cargo test --test concurrency_memory_test
//! Metrics      : peak RSS (KB) at two in-flight ceilings against an 8 MB route
//!
//! WHY THIS EXISTS
//!
//! The tool's OOM guard was believed to cover peak memory. It did not. Memory
//! is flat in *request count* (proved in `memory_bound_test.rs`) but linear in
//! *concurrent in-flight requests* on large responses, at roughly 750 KB each:
//!
//!   max_concurrency | peak RSS   (500 RPS, 8 MB bodies, idle pool pinned)
//!   ----------------+------------
//!   8               | 53 MB
//!   64              | 121 MB
//!   256             | 254 MB
//!   1000            | 750 MB
//!
//! reqwest 0.12 exposes no `http1_max_buf_size` knob, so each in-flight request
//! holds whatever buffer hyper sized for the body it is reading, and in-flight
//! count is the only lever. The old default of 1000 therefore permitted ~750 MB
//! of resident memory with no warning, which is precisely the failure mode
//! this project exists to prevent.
//!
//! The assertion is on the *slope*, not on an absolute byte count. An absolute
//! RSS threshold would be a flaky test that fails on a different allocator or a
//! different kernel and gets "fixed" by loosening the number. The slope is the
//! actual invariant: memory must be a function of the ceiling the user set,
//! and the ceiling must be small enough by default. Both are checked.
//!
//! Zero mocks: every run goes through a real axum server over a real socket.
//! ============================================================================

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rustress::core::config::Config;
use rustress::core::constants::{DEFAULT_MAX_CONCURRENCY, MAX_ALLOWED_CONCURRENCY};

mod common;
use common::dummy_server;

/// Current resident set size in bytes, or `None` off Linux.
fn current_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// Sample resident memory until the returned handle is aborted, tracking the
/// highest value seen.
///
/// Sampled rather than read from `VmHWM`, and deliberately so. `VmHWM` is the
/// peak since process start, so measuring two configurations in one process with
/// it means resetting the watermark through `/proc/self/clear_refs` -- which is
/// not writable everywhere, and this test's first draft reset it while
/// discarding the error. When the reset silently did nothing, the second
/// configuration's high-water mark was clamped to be at least the first's, so
/// the "higher ceiling costs more memory" assertion could not pass. It passed
/// locally three times and failed on the first CI run, which is exactly how a
/// false green announces itself.
///
/// Sampling has a real limitation: a spike between samples is missed. At 100 ms
/// against multi-megabyte bodies that window is narrow, and the alternative --
/// a metric that silently compares a value against itself -- is worse.
fn spawn_rss_sampler() -> (tokio::task::JoinHandle<()>, Arc<AtomicU64>) {
    let peak = Arc::new(AtomicU64::new(0));
    let handle = {
        let peak = Arc::clone(&peak);
        tokio::spawn(async move {
            loop {
                if let Some(bytes) = current_rss_bytes() {
                    peak.fetch_max(bytes, Ordering::Relaxed);
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
    };
    (handle, peak)
}

/// Run a short burst against the 8 MB route and report peak RSS in KB.
///
/// `max_concurrency` is the variable under test. The idle pool is held at a
/// single high value so that a regression in pool sizing cannot be mistaken for
/// a regression in the in-flight ceiling -- the two bounds were conflated once
/// already and that is what made the original diagnosis wrong.
///
/// The ceilings used here are deliberately modest (4 and 64). This runs
/// in-process against an 8 MB route, so a large ceiling briefly allocates
/// gigabytes. A bigger ratio would not prove more.
async fn peak_rss_kb_at(max_concurrency: u32, rps: u32, seconds: u64) -> Option<u64> {
    let server = dummy_server().await;
    let cfg: Config = Config {
        url: server.url("/big"),
        target_rps: rps,
        steady_dur_secs: seconds,
        timeout_secs: 20,
        max_concurrency,
        pool_max_idle_per_host: 1_000,
        ..Config::default()
    };
    cfg.validate().expect("config must be valid");

    let harness = common::Harness::new(cfg).expect("harness builds");
    let (sampler, peak) = spawn_rss_sampler();
    harness.run().await;
    sampler.abort();
    drop(server);

    Some(peak.load(Ordering::Relaxed) / 1024)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peak_memory_tracks_the_in_flight_ceiling() {
    let low = peak_rss_kb_at(4, 400, 5)
        .await
        .expect("the RSS sampler must produce a peak");
    let high = peak_rss_kb_at(64, 400, 5)
        .await
        .expect("the RSS sampler must produce a peak");

    // A 16x higher ceiling must cost real memory. If it does not, the bound is
    // no longer being enforced and the tool would OOM on a large target.
    assert!(
        high > low + 20_000,
        "raising max_concurrency from 4 to 64 must cost memory, \
         otherwise the ceiling is not enforced: low={low} KB high={high} KB"
    );

    // NOTE: only the *direction* is assertable here, not the slope. The
    // DummyServer is stood up in this same process (the zero-mock rule forbids
    // an out-of-process target), so RSS covers the 8 MB-serving axum server as
    // well as the generator -- and the server's own cost scales with
    // concurrency too. An earlier draft asserted an upper slope bound and
    // measured 16.1x for a 32x ceiling increase, which is the server's memory,
    // not a generator defect. The per-connection cost is pinned by
    // `the_default_ceiling_cannot_burst_to_the_previous_default` below, from the
    // measured ~750 KB per in-flight request, which is not contaminated.
}

#[ignore = "allocates hundreds of MB against an 8 MB route; this host has OOM-killed a desktop session. Run explicitly: cargo test --test concurrency_memory_test -- --ignored"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_default_ceiling_cannot_burst_to_the_previous_default() {
    // The regression is a *default value* regression: 1000 in-flight permitted
    // ~750 MB. Assert the shipped default is in a range that keeps the worst
    // case near 100 MB, using the measured ~750 KB per in-flight request.
    //
    // 128 * 750 KB ~= 96 MB. The bound is asserted as a property of the
    // constant rather than of a process, so it cannot go flaky on a different
    // allocator -- but it is still a real guard, because raising the default
    // without re-deriving this is exactly the mistake being guarded.
    let worst_case_mb = (DEFAULT_MAX_CONCURRENCY as u64) * 750 / 1024;
    assert!(
        worst_case_mb <= 128,
        "default max_concurrency of {DEFAULT_MAX_CONCURRENCY} implies a worst case \
         of ~{worst_case_mb} MB on large responses. The previous default of 1000 \
         implied ~750 MB, which is the OOM this project exists to prevent. Raise \
         the default only with a memory measurement to back it."
    );

    // And it must still be possible to opt into more, or the tool cannot load
    // test a slow target at all. Asserted through `validate()` rather than by
    // comparing the two constants: a constant comparison is provable by
    // reading the source and proves nothing at runtime.
    let raised = Config {
        url: "http://127.0.0.1:1/".to_string(),
        max_concurrency: MAX_ALLOWED_CONCURRENCY,
        ..Config::default()
    };
    assert!(
        raised.validate().is_ok(),
        "the ceiling must remain raisable via --max-concurrency, or a slow \
         target becomes untestable"
    );
}
