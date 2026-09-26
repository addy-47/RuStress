---
trigger: model_decision
description: Testing, evaluation, and benchmark standards for RuStress. Agents authoring or running tests, integration tests, or benchmarks must read this before acting.
---

# RuStress — Testing, Evaluation & Benchmark Standards

Standards for testing a load generator. **Zero mock logic is a hard requirement, not a preference.** A test that substitutes a fake HTTP client does not test a load generator; it tests the test.

---

## 1. The Zero-Mock Rule

> 🛑 **No hand-written mock types for production interfaces.**

Banned in `tests/` and `benches/`:

- `MockHttpClient`, `FakeTransport`, `StubResponder`, or any trait impl that exists only to satisfy a test
- assert-on-called ordering doubles (`expect_send_called_once`)
- hardcoded response fixtures returned from a fake instead of produced by a server
- monkey-patching or `#[cfg(test)]`-gated production branches that exist only for tests

Required instead:

| Need | Real mechanism |
|---|---|
| An HTTP target | `rustress::dummy::DummyServer` — a real axum server on a real socket |
| A specific status | A real axum route in an in-test `Router` bound to port 0 |
| A specific latency | `tokio::time::sleep` in a real handler |
| A large body | A real handler streaming real bytes |
| A malformed response | A real `TcpListener` writing raw bytes then closing |
| File-backed templates | A real `tempfile::NamedTempFile` |
| Time | `tokio::time::pause()` / `advance()`, or real short durations |

**The `dummy` server is not a mock.** It is production code — a genuine HTTP server that binds a socket and speaks HTTP. Standing it up in a test is integration testing. Standing up a `MockHttpClient` is not.

If a test needs a behaviour the `dummy` server does not expose, **add a route to the `dummy` server**. That improves the product, not just the test.

---

## 2. Testing Taxonomy

| Category | Location | Scope | Primary output |
|---|---|---|---|
| **Unit** | `#[cfg(test)] mod tests` in the target file | Pure logic: ramp maths, percentile maths, ring eviction, key-space capping, config validation, capture truncation | Pass/fail |
| **Integration** | `tests/<feature>_test.rs` | Real subsystem boundaries over real sockets: engine → executor → HTTP → metrics → export | Structural + lifecycle correctness |
| **Benchmark** | `benches/<feature>_bench.rs`, `harness = false` | Latency and allocation on the real hot path | Per-stage latency, throughput, allocs |
| **Utility** | `examples/<name>.rs` | Interactive verification against a live target | Standalone CLI |

### 2.1 What Earns A Test

- **Unit tests** cover branching logic with real edge cases: ramp boundaries, percentile on empty/singleton input, ring eviction at exactly capacity, the error-key overflow threshold, capture truncation at exactly the cap, `validate()` rejecting `max_concurrency = 0`. A test that asserts a struct default is measuring the compiler.
- **Integration tests** exercise real boundaries: a request actually crossing a socket, a real status code landing in `status_codes`, a real 500 body arriving truncated, the engine honouring a `CancellationToken`, a report round-tripping through a real CSV file. A test that calls a leaf function directly is a unit test in disguise.

### 2.2 The Direction Check

Before writing a test, identify the **production entry seam** and assert on the **observable exit**. Asserting on the consumer of a value rather than the producer proves nothing.

- Wrong: assert `executor::clean_error_msg` strips a prefix → proves nothing about a real request.
- Right: stand up a server that closes the connection, run the engine against it, assert `snapshot.error_counts` contains a connection error → proves the whole path.

---

## 3. Crash-Safety Tests — MANDATORY

> 🛑 **These exist because the tool crashed the author's machine. They must not be deleted.**

The tool OOM'd its host by retaining every request result — including full error response bodies — in an unbounded `Vec`. Every bound added to prevent a repeat needs a test that fails if the bound is removed.

Required coverage:

| Property | Test shape |
|---|---|
| Result retention is bounded | Push 100k results into a `ResultLog` with capacity 16; assert `len() == 16` and `dropped_from_front() == 99_984` |
| Error key space is bounded | Feed 3200 unique error strings; assert `error_counts.len() == MAX_TRACKED_ERROR_KEYS + 1` and the overflow bucket holds the remainder |
| Response body capture is capped | Serve a body larger than `MAX_CAPTURED_BODY_BYTES`; assert the captured body is capped and carries the truncation marker |
| A huge body does not exhaust memory | Serve a 64 MB body at concurrency 50; assert RSS stays bounded — **this is the regression test for the crash** |
| Concurrency ceiling holds | Saturate a slow target; assert `snapshot.dropped_scheduled > 0` and that in-flight never exceeds `max_concurrency` |
| Terminal is restored | Assert the TUI teardown path runs on error return and on unwind — see below |
| Config rejects deadlock | `validate()` must reject `max_concurrency = 0` |

**Terminal restoration test.** `TerminalGuard` cannot be unit tested against a real TTY in CI. Assert the structural invariant instead: that no module outside `tui/guard.rs` calls into `crossterm::terminal`, and that `Cargo.toml` sets no `panic = "abort"` in any profile. A grep-based test is legitimate here — the property is architectural.

---

## 4. Benchmark Standards

> 🛑 **A load generator's own performance is the product. If it cannot saturate a NIC, it cannot measure one.**

- **Always `--release`.** Debug builds produce latency numbers up to 7× worse and will make a correct implementation look broken.
- **Sequential, never concurrent.** One configuration at a time. Parallel benchmarks contend for cores and invalidate every comparison.
- **`harness = false` with a custom `fn main()`.** No `#[bench]`, no nightly.
- **Report per-stage decomposition, not just totals.** For the request path, separate: request construction, template rendering, time on the wire, response draining, stats folding. A regression hidden inside a single E2E number is a defect you will ship.
- **Measure allocations on the hot path.** `PreparedRequest::build` and `stats.record` are called per request. Report allocs/request so a reintroduced per-request allocation is visible.
- **Assert an allocation ceiling**, not just a latency number. `assert!(allocs_per_request <= 16)` catches a reintroduced per-request `Environment::new()` immediately; a latency threshold often will not.
- **Parametrise via CLI args or `env!`** so CI can sweep configurations without recompiling.
- **Benchmarks must not require an external target.** Use `DummyServer` on an ephemeral port.

### 4.1 Mandatory Header

Every `tests/*.rs`, `benches/*.rs`, and `examples/*.rs` file:

```rust
//! ============================================================================
//! <filename> — <one-line description>
//! ============================================================================
//! Category     : [Integration Test | Benchmark | Utility Tool]
//! Component    : <target module or subsystem>
//! Prerequisites: <required env vars, services, or data>
//! Execution    : <exact cargo command>
//! Metrics      : <recorded operational/quality metrics>
//! ============================================================================
```

---

## 5. Execution Standards

- **Sequential execution only.** Run one suite at a time; CPU and I/O contention invalidates both timing and any memory measurement.
- **Assert semantic correctness, not exit code.** `Ok(())` from a test that measured `p99 == 0` is a false green.
- **Ask what a passing test proves.** If the production path it exercises were deleted, would it still pass? If yes, the test is measuring nothing.
- **A test that passes while production is broken is worse than no test.** Before accepting, state the expected value, then confirm the assertion would fail on a plausible defect.
- **Mutation-validate every crash-safety test.** Remove the bound, confirm the test goes red, restore it. An unvalidated crash-safety test is a comment, not a guard.
- **Never lower a threshold to make a test pass.** Investigate the regression instead.

---

## 6. Lifecycle Discipline: `/create-test` → `/test` → `/mutate`

1. **Construction (`/create-test`)** — trace the production entry seam, verify it is reachable without mocks, complete the Phase 2b False-Green table.
2. **Execution (`/test`)** — define exact success criteria before running; read output for silent wrongness beyond exit code 0.
3. **Validation (`/mutate`)** — seed the False-Green defects into production code, prove the suite goes red, revert, prove green.
