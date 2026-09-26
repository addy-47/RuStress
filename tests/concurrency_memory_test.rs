//! ============================================================================
//! concurrency_memory_test.rs — the in-flight ceiling bounds worst-case memory
//! ============================================================================
//! Category     : Integration Test
//! Component    : core::constants (DEFAULT_MAX_CONCURRENCY)
//! Prerequisites: none
//! Execution    : cargo test --test concurrency_memory_test
//!
//! WHY THIS EXISTS
//!
//! Peak memory on large responses is a function of **concurrent in-flight
//! requests**. Each holds a hyper HTTP/1 read buffer grown to service the body
//! it is reading, and reqwest 0.12 exposes no `http1_max_buf_size` knob, so the
//! in-flight count is the only lever. Measured at 500 RPS against 8 MB bodies
//! with the idle connection pool held constant, so the two bounds could not be
//! confused:
//!
//!   max_concurrency | peak RSS
//!   ----------------+-----------
//!   8               | 53 MB
//!   64              | 121 MB
//!   256             | 254 MB
//!   1000            | 750 MB
//!
//! The default of 1000 therefore permitted ~750 MB of resident memory with no
//! warning. It is now 128, capping the worst case near 96 MB.
//!
//! A RELATIONSHIP ASSERTION IS NOT MADE HERE, AND ITS ABSENCE IS DELIBERATE.
//!
//! An earlier version of this file asserted that raising the ceiling costs
//! more memory, measuring inside the test process. It does not hold there, and
//! the failure is instructive rather than flaky:
//!
//!   low (ceiling 4)  = 101820 KB
//!   high (ceiling 64) =  93284 KB
//!
//! The *higher* ceiling measured *lower*. The zero-mock rule requires the target
//! server to be real, which means it runs in this same process, and a server
//! serving 8 MB bodies allocates enough to swamp the generator's contribution.
//! The two costs are not separable by sampling one process. Two earlier
//! attempts failed for the same underlying reason: one read `VmRSS` at a single
//! instant and flapped; the other read the monotonic `VmHWM` and reset it via
//! `/proc/self/clear_refs`, which is not writable in every environment, so the
//! second measurement was clamped to be at least the first and the assertion
//! compared a value with itself. All three passed locally and failed on CI.
//!
//! The relationship above was measured out-of-process instead, by running the
//! release binary against a separately-started server. That measurement is
//! reproducible on demand:
//!
//!     cargo run --release --example bounded_memory
//!
//! What is asserted here is the part that *is* deterministic and that protects
//! the default: the shipped ceiling implies a worst case inside a sane budget.
//! It allocates nothing, runs anywhere, and fails loudly if the default is ever
//! raised back toward the value that caused the problem.
//!
//! Zero mocks: no target, no sockets, no traffic.
//! ============================================================================

use rustress::core::config::Config;
use rustress::core::constants::{DEFAULT_MAX_CONCURRENCY, MAX_ALLOWED_CONCURRENCY};

/// Measured marginal cost of one in-flight request against a large body.
///
/// Derived from the table above: (750 MB - 121 MB) / (1000 - 64) ~= 750 KB.
/// Small bodies are unaffected, which is why the same 1000 in-flight run against
/// a small-body route peaks at 7.3 MB.
const KB_PER_INFLIGHT_REQUEST: u64 = 750;

/// The worst-case resident memory the shipped default can permit, in MB.
const WORST_CASE_BUDGET_MB: u64 = 128;

#[test]
fn the_default_ceiling_cannot_burst_to_the_previous_default() {
    let worst_case_mb = (DEFAULT_MAX_CONCURRENCY as u64) * KB_PER_INFLIGHT_REQUEST / 1024;

    assert!(
        worst_case_mb <= WORST_CASE_BUDGET_MB,
        "default max_concurrency of {DEFAULT_MAX_CONCURRENCY} implies a worst case \
         of ~{worst_case_mb} MB on large responses. The previous default of 1000 \
         implied ~750 MB, which is the OOM this project exists to prevent. Raise \
         the default only with an out-of-process memory measurement to back it."
    );
}

#[test]
fn the_ceiling_stays_raisable() {
    // A default that cannot be raised is its own bug: a slow target needs more
    // requests in flight to sustain a given RPS, and a ceiling too low to lift
    // makes the generator untunable.
    //
    // Asserted through `validate()` rather than by comparing the two constants,
    // which would be provable by reading the source and would prove nothing.
    let raised = Config {
        url: "http://127.0.0.1:1/".to_string(),
        max_concurrency: MAX_ALLOWED_CONCURRENCY,
        ..Config::default()
    };
    assert!(
        raised.validate().is_ok(),
        "the ceiling must remain raisable via --max-concurrency"
    );
}

#[test]
fn a_generous_ceiling_is_still_valid_but_would_burst() {
    // Documents the shape of the trade-off rather than forbidding the value. A
    // user may legitimately want 1000 in-flight against a 50 ms endpoint, where
    // the real cost is bounded by throughput rather than by response size. The
    // point is that the cost is knowable in advance rather than discovered when
    // the machine dies.
    let aggressive = Config {
        url: "http://127.0.0.1:1/".to_string(),
        max_concurrency: 1_000,
        ..Config::default()
    };

    assert!(
        aggressive.validate().is_ok(),
        "a high ceiling must be allowed; the generator sheds and counts rather \
         than queueing, so under-provisioning is visible"
    );

    let implied_mb = 1_000 * KB_PER_INFLIGHT_REQUEST / 1024;
    assert!(
        implied_mb > WORST_CASE_BUDGET_MB,
        "1000 in-flight is expected to exceed the {} MB budget (~{implied_mb} MB); \
         if this constant drifts, the table in the module docs is wrong too",
        WORST_CASE_BUDGET_MB
    );
}
