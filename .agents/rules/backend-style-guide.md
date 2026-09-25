---
trigger: model_decision
description: RuStress code style guide and engineering standards for the Rust crate (`src/`). Agents doing write operations on Rust code must read this before modifying code.
---

# RuStress — Code Style Guide & Engineering Standards

Standards for the RuStress crate (`src/`). This is a **single-crate** project: one `Cargo.toml` publishing a library (`rustress`) and a binary (`rustress`). There is no cargo workspace and no inter-crate boundary to design around.

**The governing constraint:** RuStress is a load generator. It is trusted with a target URL, a request rate, and a concurrency ceiling, and it runs on the same machine as the person using it. Two failure modes are unacceptable and outrank every performance or elegance concern:

1. **It must never exhaust host memory.** A load tool that OOMs the user's machine destroys the machine, not just the run.
2. **It must never leave the terminal unusable.** A TUI that exits without restoring the terminal costs the user their shell.

Every rule below is in service of correctness, bounded resource use, and measurement validity.

---

## 1. Module Organization & File Boundaries

- **Domain over type:** Group code by domain (`runner/executor.rs`, `metrics/collector.rs`), never by Rust construct (`models.rs`, `handlers.rs`).
- **Single responsibility:** 1 responsibility per file. If a file cannot be described in 1 sentence, split it.
- **File size ceiling:** Flag and justify files exceeding ~600 lines. `src/tui/views/runner.rs` is the known largest and is a candidate for splitting.
- **`mod.rs` / `lib.rs`:** module declarations, re-exports, and **subsystem-level constants only**. Zero business logic.
- **Sibling imports:** use `super::` for siblings inside a domain, `crate::<domain>::` for cross-domain. Never `crate::<sibling>`.
- **Visibility:** `pub(crate)` over `pub` unless it is part of the published API surface (`src/lib.rs` re-exports) or needed by `tests/`.
- **Derives:** `#[derive(Debug, Clone, PartialEq, Eq)]` on domain types. `Clone` on a type held per-request is a cost decision, not a default — justify it.
- **No file may be named the same as its parent module** (`cli/cli.rs` is banned; use `cli/args.rs`). Clippy enforces this.

### 1.1 File Grammar Order

Every `.rs` file, top to bottom:

1. `use` statements — grouped: `std`, external crates, `crate::`, `super::`.
2. File-local constants and type aliases.
3. `struct` / `enum` declarations.
4. `impl` blocks — constructors, public methods, private methods.
5. Free helper functions.
6. `#[cfg(test)] mod tests`.

---

## 2. Constant Hierarchy (CRITICAL)

Never scatter magic numbers through the engine. All tunables live in `core/constants.rs`, grouped by concern with a comment explaining *why the bound exists*:

| Group | Purpose | Examples |
|---|---|---|
| Request-body capture | Bound memory taken from an untrusted target | `MAX_CAPTURED_BODY_BYTES`, `MAX_DRAINED_BODY_BYTES` |
| Result retention | Bound memory taken by result retention | `RESULT_RING_CAPACITY`, `MAX_TRACKED_ERROR_KEYS` |
| Scheduling | Bound resource ceilings and their validation floors | `MIN_ALLOWED_CONCURRENCY`, `MAX_ALLOWED_CONCURRENCY`, `MAX_ALLOWED_USERS`, `MAX_ALLOWED_RPS` |
| Defaults | User-configurable defaults | `DEFAULT_TARGET_RPS`, `DEFAULT_TIMEOUT_SECS` |
| Connection pool | Socket retention | `MAX_IDLE_CONNS` |
| Telemetry cadence | UI update intervals | `STATS_UPDATE_INTERVAL_MS` |
| Histogram bounds | Latency recording range | `HISTOGRAM_LOW_US`, `HISTOGRAM_HIGH_US`, `HISTOGRAM_SIGFIGS` |

**A new bound is not finished until it has a name, a comment justifying the value, and a validation rule in `Config::validate`.**

---

## 3. Bounded Memory — MANDATORY

> 🛑 **This is the rule that prevents the tool from crashing the user's machine.**

RuStress holds state that grows with request count. Every such structure must be bounded by configuration, never by traffic volume.

**Banned — any collection whose size is a function of requests executed:**

| Banned | Required instead |
|---|---|
| `Vec`/`VecDeque` that only grows | Fixed-capacity ring buffer — see `runner/result_log.rs` |
| `HashMap`/`IndexMap` keyed by target-supplied text | Bounded key space with an overflow bucket — see `metrics::bounded_error_key` |
| `String` holding a response body | Cap at capture limit, mark truncation explicitly |
| `JoinHandle` vector accumulated per request | Acquire the concurrency permit **before** spawning; reap finished handles |
| `mpsc::unbounded_channel` on a per-request path | Bounded channel, or emit on a fixed cadence only |

**Rules:**

- **Aggregate accuracy is O(1) in memory.** Counters live in atomics; latency lives in HDR histograms. Never keep per-request samples to compute a statistic that a histogram already provides.
- **Per-request samples are for report export only**, and they are a bounded ring. When the ring overflows, the drop count is reported — never silently hidden.
- **Anything read from a response is attacker-controlled.** A `500` carrying a 2 GB error page is normal. Cap every read and mark truncation (`executor::TRUNCATION_MARKER`).
- **Anything keyed by error text is target-controlled.** A target returning a unique error string per request must not be able to grow a map. Cap the key space and fold the remainder.
- **Spawning is bounded, not just execution.** A semaphore acquired *inside* a spawned task bounds concurrency but not task count. Acquire before spawn.

**Before adding any per-request state, answer:** what is the worst-case resident memory at 1M requests, and what bounds it? If there is no answer, do not add it.

---

## 4. Hot Path Discipline

The request path is `runner/engine.rs` → `runner/executor.rs`. It runs at the configured RPS and is the only place where per-request cost matters.

- **Decide once at construction, not per request.** HTTP method parsing, header-name lowercasing and validation, `content-type` detection, `@file` body loading, and template *compilation* all belong in `PreparedRequest::new`. `PreparedRequest::is_templated()` must be `false` for a request with no template directives.
- **No template engine construction per request.** Building a `minijinja::Environment` and re-parsing a template per request is the single most expensive mistake available in this codebase. Render, never recompile.
- **No `to_lowercase()` / `to_string()` / `format!` in the hot path** unless the result is genuinely per-request-unique. Reuse precomputed `HeaderName` values.
- **No full-body materialisation.** Stream with `Response::chunk()` and count bytes as they pass. `Response::bytes()` and `Response::text()` are banned in the executor.
- **Counting bytes from the stream, not from `content_length()`.** `content_length()` is `None` for chunked responses, so trusting it silently reports zero throughput.
- **Hot path is allocation-light but not allocation-free-by-fiat.** Do not add a cache layer without a measurement showing the allocation is on the path.

---

## 5. Load Generation Semantics

Getting these wrong produces numbers that look like results and are not.

- **Open loop (RPS) keeps wall-clock time.** Requests are scheduled against a timeline. When concurrency is saturated the request is **dropped and counted** (`RunStats::record_scheduled_drop`), never queued. Queueing converts a generator limit into apparent target latency — coordinated omission.
- **Closed loop (Users) is bounded by `num_users`,** which is validated against `MAX_ALLOWED_USERS`. Do not introduce a second, independent concurrency control in this path.
- **A non-zero `dropped_scheduled` invalidates the run's latency figures.** It must be surfaced in the TUI (`render_shed_notice`) and in the headless summary, not only in a report file.
- **Service time and total latency are different measurements.** `service_time` is request start to response; `latency` is scheduled time to response and therefore includes queue wait. Never conflate them, and never report one as the other.
- **Ramp profiles are pure functions** of elapsed time (`runner/ramp.rs`) so they can be unit tested without a clock. Keep them side-effect free.
- **Drain is mandatory.** After the schedule ends, in-flight requests must complete before results are summarised. Cancellation must not abandon counted work silently.

---

## 6. Function Standards & Code Cleanliness

- **Function line cap (soft):** 50 lines without documented justification.
- **Docstrings:** exactly one `///` per function stating what it takes and what it returns. Zero narrative comments inside bodies; runtime traces belong in `tracing::`.
- **No step-comment sequences.** `// 1. …` `// 2. …` means each step belongs in a named private function.
- **No toggle functions.** `start()` starts; it does not contain `if cond { start } else { stop }`.
- **Struct bundling past 5 arguments.** `StatsCollector::add` takes one `ExperimentResult`, not nine positional parameters — adding a measurement must not be able to silently reorder call sites.
- **Zero `#[allow(...)]`.** No `too_many_arguments`, `dead_code`, or `unused_variables` suppressions.
- **Zero `_` masking** of unused variables, except genuine RAII guards (`_permit`, `_guard`).
- **Errors are typed at the boundary.** `thiserror` enums for library errors, `anyhow` for context in the CLI layer. No `.unwrap()` or `.expect()` in production paths — `expect` is permitted only where an invariant is enforced at construction (e.g. HDR histogram bounds, which are compile-time constants).
- **No silent error swallowing.** No `let _ = fallible()`. Log with `tracing::warn!` or propagate.
- **No fallback chains.** One deterministic path per operation. If it fails, report it.
- **`panic = "abort"` is banned in every profile.** Unwinding is what runs `Drop`, and `Drop` is what restores the user's terminal.

---

## 7. Terminal Safety — MANDATORY

> 🛑 **A TUI that leaves the terminal in raw mode costs the user their shell.**

- **All terminal takeover lives behind `tui::TerminalGuard`.** No other module may call `enable_raw_mode`, `EnterAlternateScreen`, or `EnableMouseCapture`.
- **Teardown happens in `Drop`,** never as a trailing statement in the run function. A `?`, a panic, or an early return must all restore the terminal.
- **Any new `unsafe` or FFI-adjacent terminal call must go through the guard.**
- **In headless mode the terminal is never touched.** If a code path can run without a TTY, it must not enable raw mode.
- **Stdout is not a log sink.** `println!` is for user-facing CLI output only; diagnostics go through `tracing::`.

---

## 8. Concurrency & Threading

- **Async runtime tasks for I/O; `spawn_blocking` for blocking work.** Synchronous file reads and `std::thread::sleep` on a runtime worker stall every other request.
- **Cancellation via `CancellationToken`,** checked at loop boundaries and in `select!` arms. Never busy-wait on a flag.
- **Channels over shared mutexes** for cross-task communication. `parking_lot` mutexes guard small state (`StatsCollector` maps, `LatencyHistogram`), never held across `.await`.
- **Atomics use `Relaxed` for counters** that are only aggregated, and a stronger ordering when correctness depends on it. Do not pay for `SeqCst` by default.
- **Canonical lock order:** no nested locks in the hot path. `StatsCollector::add` takes each map lock once, in a fixed order, and releases before the next.

---

## 9. Testability Seams

- **The public API is the test surface.** `tests/` may only use `rustress::*` re-exports.
- **No mocks.** See `.agents/rules/testing-style-guilde.md`. The `dummy` axum server is a real HTTP server, so using it is not mocking — a hand-rolled `MockHttpClient` is.
- **Seams that must exist for zero-mock testing:** `PreparedRequest::new` (pure, sync), `run_rps`/`run_users` (take a `CancellationToken`), `RunStats` (observable counters), `ResultLog` (observable eviction), `TemplateEngine` (real minijinja over a real filesystem).
- **Never introduce a module-level `static` that forms a black box** an upstream actor cannot feed or a test cannot observe.

---

## 10. Verification Before Declaring Done

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo test --doc
```

All four must be clean. `cargo doc --no-deps` must build without warnings before publishing.

---

## 11. Test & Benchmark Placement

| Category | Location | Command | Access |
|---|---|---|---|
| Unit | `#[cfg(test)] mod tests` in the target file | `cargo test --lib` | private + public |
| Integration | `tests/<feature>_test.rs` | `cargo test --test <name>` | public API only |
| Benchmark | `benches/<feature>_bench.rs`, `harness = false` | `cargo bench` | public API |
| Utility | `examples/<name>.rs` | `cargo run --release --example <name>` | public API |

Full standards: `.agents/rules/testing-style-guilde.md`.
