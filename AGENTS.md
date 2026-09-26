# AGENTS.md — RuStress Workspace Rules

---

## 1. MANDATORY RULE: AGENTS.md Sync Hook

> 🛑 **After every completed task, append one concise bullet to Section 5 describing what changed.**
>
> Do not simultaneously write to `docs/` or any other file — `AGENTS.md` is the only target. When Section 5 approaches **30 bullets**, compact it to the highest-level milestones and archive the detail to `docs/recent_work.md`.

---

## 2. Project Map

RuStress is a **single-crate** Rust load generator. One `Cargo.toml` publishes a library (`rustress`) and a binary (`rustress`). There is no cargo workspace.

| Path | Purpose | Rules |
|---|---|---|
| `src/core/` | Config, `ExperimentResult`, `StatsSnapshot`, errors, all tunable constants | Leaf module. No subsystem knowledge. Every memory/scheduling bound gets a name here. |
| `src/metrics/` | `StatsCollector`, HDR `LatencyHistogram`, percentiles | O(1) memory in request count. Map key spaces must be bounded. |
| `src/templates/` | Per-request value injection, file cache | Never construct an `Environment` per request. |
| `src/runner/` | Scheduling (`engine`), request build (`request`), execution (`executor`), ramp maths, client | **The hot path.** Decide once at construction. |
| `src/runner/result_log.rs` | Fixed-capacity retention ring | Bounded by `RESULT_RING_CAPACITY`. Evictions are counted and reported. |
| `src/tui/` | ratatui dashboard, theme, event loop | All terminal takeover behind `TerminalGuard`. |
| `src/export/` | CSV / JSON / summary writers | Consume the bounded result log only. |
| `src/dummy/` | Built-in axum target server | **Production code, not a test fixture.** Tests stand this up for real. |
| `src/cli/` | Argument definitions and subcommand dispatch | Thin shell over the library. |
| `tests/` | Integration tests, public API only | Zero mock logic. `<feature>_test.rs`. |
| `benches/` | Performance benchmarks | `harness = false`. Release only. |
| `scripts/safe-cargo.sh` | Memory-capped cargo wrapper | Mandatory for every cargo invocation — see 4.4. |
| `examples/` | Runnable dev utilities | Public API only. |
| `.agents/rules/` | Role + style-guide instructions | Read the relevant file before acting in that role. |

**Publishing:** `rustress` on crates.io. CI runs test/clippy/doc on tag push; publish fires on GitHub release.

---

## 3. Execution & Testing Invariants

1. **Sequential execution only.** Run test suites and benchmarks one at a time — CPU, memory, and I/O contention invalidate both timing and any memory measurement.
2. **Benchmarks and perf tests are always `--release`.** Debug builds report latency up to 7× worse and will make correct code look broken.
3. **Full-suite baseline is ~1s.** Use a timeout when running the load generator against a live target; a hung target plus a 30s request timeout can stall a run.
4. **External-target tests are opt-in.** Any test requiring a target the machine does not control must be `#[ignore]`d and run manually: `cargo test -- --ignored`.
5. **Verify before declaring done:**
   ```bash
   cargo fmt --check
   ./scripts/safe-cargo.sh clippy --all-targets -- -D warnings
   ./scripts/safe-cargo.sh test --all-targets
   ./scripts/safe-cargo.sh test --doc
   ```
   All four must be clean. `cargo doc --no-deps` must be warning-free before publishing.

---

## 4. Non-Negotiable Invariants

> 🛑 **These two failure modes are unacceptable and outrank every performance and elegance concern. The tool has already committed both.**

### 4.1 Bounded Memory (P0)

The tool previously OOM'd the author's machine by retaining every request result — including full error response bodies — in an unbounded `Vec`. Assume any change that grows state with request count is a repeat of that bug until proven otherwise.

- Every per-request structure has a **configured ceiling**. Aggregate accuracy is O(1) in memory (atomics + HDR histograms); per-request samples live in a fixed-capacity ring and their evictions are **counted and reported**, never hidden.
- **Anything read from a response is target-controlled.** A `500` carrying a 2 GB error page is normal. Cap every read; mark truncation explicitly.
- **Anything keyed by error text is target-controlled.** A target returning a unique error string per request must not be able to grow a map. Cap the key space, fold the remainder into an overflow bucket.
- **Spawning is bounded, not just execution.** A semaphore acquired *inside* a spawned task bounds concurrency but not task count. Acquire the permit **before** spawning.
- **Before adding per-request state, state the worst-case resident memory at 1M requests and what bounds it.** If there is no answer, do not add it.

### 4.2 Terminal Safety (P0)

A TUI that exits without restoring the terminal costs the user their shell.

- All terminal takeover lives behind `tui::TerminalGuard`. No other module may call `enable_raw_mode`, `EnterAlternateScreen`, or `EnableMouseCapture`.
- Teardown is in `Drop` — never a trailing statement in the run function. `?`, panic, and early return must all restore the terminal.
- **`panic = "abort"` is banned in every profile.** An abort skips destructors and reintroduces the broken terminal.
- Headless mode never touches the terminal.

### 4.3 Measurement Validity

- **Open loop (RPS) keeps wall-clock time.** On saturation, requests are **dropped and counted**, never queued. Queueing converts a generator limit into apparent target latency.
- **A non-zero `dropped_scheduled` invalidates the run's latency figures.** It must be visible in the TUI and the headless summary, not only in a report file.
- `service_time` (request start → response) and `latency` (scheduled → response, includes queue wait) are different measurements. Never report one as the other.
- Count response bytes from the stream. `content_length()` is `None` for chunked responses, so trusting it silently reports zero throughput.

### 4.4 Build Memory Safety (P0)

> 🛑 **This host has 15 GB RAM and 8 cores. An uncapped release build of this dependency tree OOM-killed the desktop session twice.**

Nothing in Cargo, rustc, or clippy enforces a memory ceiling. Parallel codegen units are a *memory* multiplier, not just a speed one, and the LTO link adds a large single-process spike.

- **Every cargo invocation MUST go through `scripts/safe-cargo.sh`**, which imposes a hard cgroup v2 `MemoryMax` (default 6G) so an over-budget build is killed instead of taking the machine with it. A killed build is a failed build; a killed machine is a lost session.
- **`.cargo/config.toml` pins `jobs = 2`.** Do not raise it. `jobs = 8` is what caused the crash.
- **Overrides, in order of preference:** use `cargo check` over `build --release`; lower `SAFE_CARGO_JOBS`; only then raise `SAFE_CARGO_MEM`, and only after confirming free headroom with `free -g`.
- **Never run a release build, a full-dependency compile, and the load generator concurrently.** Verification is sequential, always.
- **Exit code 137 or 143 from the wrapper is the failsafe firing, not a bug.** Re-run with a lower job count rather than removing the cap.
- **Subagents inherit this rule.** Any agent that runs cargo must use the wrapper, and must not raise the job count or memory cap on its own initiative.

### 4.5 HARD GATE: Code Modification Gate

> 🛑 **Before ANY write task — production, test, or bench — read the matching files in `.agents/rules/`.**

| Working in | Read |
|---|---|
| `src/` production code | `backend-style-guide.md` + `backend-engineer.md` |
| `tests/`, `benches/`, `examples/` | `testing-style-guide.md` + `test-engineer.md` |
| Reviewing or gating | `qa-engineer.md` |
| New features, module boundaries, dependencies, public API | `system-architect.md` |
| `/` `frontend-engineer.md` — **not applicable.** This project has no web frontend. The TUI is Rust. |
| Before any cargo invocation | `scripts/safe-cargo.sh` — see 4.4 |

---

## 5. Recent Work

- **Collapse to a single crate:** 8 workspace crates merged into one `Cargo.toml` publishing a lib + bin. Imports rewritten from `rustress_*::` to `crate::<domain>::`, siblings to `super::`.
- **Build memory safety (host OOM-killed twice):** every cargo invocation now goes through `scripts/safe-cargo.sh`, which imposes a hard cgroup v2 `MemoryMax` (6G default) so an over-budget build is killed instead of taking the session with it. `.cargo/config.toml` pins `jobs = 2` (was 8). See 4.4.
- **P0 memory bounds (the original crash):** unbounded `Vec<ExperimentResult>` → fixed-capacity `ResultLog` ring (50k, evictions counted); unbounded `IndexMap<String, u64>` error map → 64-key budget + overflow bucket; response bodies streamed with `Response::chunk()` and captured under a 2 KB cap with an explicit truncation marker. Bodies are now drained to EOF so connection reuse — and therefore the measured service time — is not corrupted by a fresh handshake per request.
- **P0 unbounded task spawn:** `run_rps` accumulated a `JoinHandle` per request forever. The concurrency permit is acquired **before** spawning via `try_acquire_owned`; on saturation the request is dropped and counted in `dropped_scheduled`, surfaced in the TUI and the headless summary.
- **P0 terminal safety:** added `tui::TerminalGuard` (RAII) owning raw mode, alternate screen, and mouse capture. The guard is constructed *before* the alternate screen is entered, so a failed `execute!` still tears down. Removed `panic = "abort"` so `Drop` runs on panic.
- **P0 percentile scale bug (every latency figure was wrong):** `PercentileExt` passed `50.0`/`90.0`/`99.0` to `hdrhistogram::value_at_quantile`, which takes a **fraction 0.0-1.0**. Every value clamped to the 100th percentile, so p50, p90, p95 and p99 all reported the maximum observed latency. Fixed, plus a regression test asserting percentiles strictly increase on a known spread.
- **P0 headless summary under-reported by ~50%:** the monitor loop took one snapshot per 200ms tick from a channel the engine wrote every 100ms, so the newest frame was never read — 973 requests reported where 2000 executed. The loop now drains all pending frames and the final summary reads counters directly after drain.
- **P0 drain barrier abandoned queued tasks:** `inflight` was incremented inside each request task, so a spawned-but-unpolled task was invisible to `await_drain` and dropped at shutdown. Inflight is now admitted at dispatch (strictly before `tokio::spawn`) and released by an `InflightGuard` `Drop` impl that also survives a panicking task.
- **P0 `Config::validate()` was never called:** every bound in `Config` was inert, so `--users 100000000` reached `run_users` unbounded. Validation now runs inside `LoadEngine::new`, so library embedders cannot bypass it.
- **P0 latencies above the histogram ceiling vanished:** `record` returns `Err` and stores nothing above `HISTOGRAM_HIGH_US`, so the worst samples were absent from every percentile while still being counted in `requests` and `fail`. Switched to `saturating_record`.
- **P1 config file was silently overwritten:** `body` and `out_prefix` were assigned unconditionally from `Option`s, so `--config load.toml` sent bodyless requests and exported nothing while reporting a clean run. Method/body/out are now applied only when actually passed, and a named-but-unreadable config file is a hard error rather than a warning that silently launches the TUI.
- **P1 silent method and header fallbacks:** an invalid `--method` became `GET` (measuring a read path when a write path was asked for) and a malformed header name was dropped, so an auth header could vanish with no diagnostic. Both are now construction-time errors.
- **P1 measurement-validity fixes:** the TUI summary and the summary JSON were derived from the 50k retention ring rather than the authoritative counters, so long runs reported a second, different set of numbers. Both now read `StatsSnapshot`, and the summary JSON carries `dropped_scheduled`, `results_evicted_from_ring` and `measurement_valid`.
- **P1 hot path:** `PreparedRequest` moves method parsing, header validation, `content-type` detection and `@file` loading to construction; `Config` is shared via `Arc` instead of deep-cloned per request; a dead second histogram mutex (`total_time`) removed; the TUI redraws on change rather than at 100 Hz; the 50k result ring is only materialised when an export needs it.
- **Packaging:** crates.io metadata, `rustfmt.toml`, `exclude`; untracked a 6 MB compiled binary that was git-tracked at the repo root and would have shipped in the `.crate`; added the `/big` regression route to the dummy server so the memory bound is reproducible with the repo's own tooling. `cargo clippy --all-targets` is clean.
- **P1 a mid-body error was reported as a success:** `executor` derived `success` from the status code alone and discarded the stream error, so a `200` whose body failed counted in `success` and not in `fail` while `error_counts` disagreed. A run could report 100% success against a target returning unusable responses. `StatsCollector` also branched on `error.is_some()`, which erased a *known* status from `status_codes`; status and error are now recorded independently.
- **Test suite (`tests/`, 9 files, 52 integration tests + 28 unit tests, zero mocks):** every test stands up a real target — the `DummyServer` on an ephemeral port, in-test `axum` routers, a raw `TcpListener` for truncated responses, real `tempfile` configs. `memory_bound_test` is the OOM regression guard and asserts a *scaling* property (two runs at identical concurrency and different request counts must cost the same resident memory) plus a positive control proving the `/proc` RSS probe can see retention at all. `scheduling_test` pins open-loop rate accuracy, saturation honesty and the in-flight high-water mark. `structural_safety_test` asserts `enable_raw_mode`/`EnterAlternateScreen`/`EnableMouseCapture` appear only in `guard.rs` and that no profile sets `panic`. **Seven mutations were seeded and reverted to prove the suite goes red** (percentile scale, `content_length` over stream, unbounded ring, removed `validate()` call, config-file erasure, error-key budget, always-truncate).
- **Benchmarks (`benches/hot_path_bench.rs`, `benches/aggregation_bench.rs`, `harness = false`):** per-stage decomposition of the request path and the aggregation path, with hard allocation ceilings rather than latency thresholds. Both refuse to run under `debug_assertions`, because `cargo test --all-targets` includes bench targets and a `harness = false` bench's `main` *is* the test entry point — a benchmark that printed debug numbers would be evidence-shaped and meaningless.
- **Production seams added for testability (3 changes, all in service of a real test):** `DummyServer::router()` exposes the route table so a caller can bind an ephemeral port and read back the real address — `run()` takes a port and reports none, which made port-0 impossible; `/sized?bytes=N` serves an exactly-sized 500 body (the capture cap is a boundary, and only an exact-size fixture can prove the off-by-one); `executor::TRUNCATION_MARKER` is now `pub` so a report consumer can detect truncation from the constant rather than a drifting literal. `/sized` takes a **query** parameter, not a path parameter — see the finding below.
- **KNOWN GAP (P2) — `max_concurrency` is inert in Users mode.** `runner/engine.rs` acquires a semaphore permit only in `run_rps`; `run_users` bounds concurrency by `num_users` alone. `Config::max_concurrency` is documented as "max concurrent in-flight requests (prevents OOM/Task explosion)", so a user setting it in Users mode gets no ceiling they asked for. Bounded by `MAX_ALLOWED_USERS`, so this is a semantic gap, not a memory-safety one. No test asserts the current behaviour, deliberately: enshrining it would make the bug look intended.
- **KNOWN GAP (P3) — a per-request `minijinja::Environment::new()` on the hot path.** `templates/engine.rs::execute_str` constructs an `Environment`, registers six functions and re-parses the template on *every* call, and it is the path the engine takes per request for a templated URL, body or header. This directly contradicts backend-style-guide §4 ("no template engine construction per request"). `PreparedRequest::new` pre-decomposes the request but `execute` is a convenience method that recompiles, so the pre-parsing buys nothing on the templated path. `benches/hot_path_bench.rs` reports the per-op allocation count and prints this as an open finding; its allocation ceiling is deliberately loose because the current cost is the defect.
- **KNOWN GAP (P3, characterisation only) — empty inline body and empty `@file` body disagree.** `PreparedRequest::new` maps `Some("")` to "no body" (no `content-type` injected) but an empty `@file` to `Some("")` — a body that carries a `content-type`. `tests`-adjacent unit test `an_empty_body_file_is_still_treated_as_carrying_a_body` records the behaviour and labels it characterisation, not endorsement.
- **ENVIRONMENT (host, not code) — the local crates.io cache holds a `matchit-0.7.3` whose source implements the 0.6-era `:name` route syntax.** Every axum `{param}` route 404s on this machine, including when the router is called directly through `tower` with no socket involved. Verified by bisection: `/t/:n` matches, `/t/{n}` does not. This is why `/sized` takes a query parameter. Any future axum route in this crate using `{param}` will appear broken here and work on a clean registry — re-verify the cache before debugging the route.
- **MEASURED (P0, unfixed) — peak RSS has a ceiling of ~800 MB on large bodies, and it is the connection pool, not the request count.** `client.rs` sets `pool_max_idle_per_host(min(max_concurrency, MAX_IDLE_CONNS))`, conflating two different bounds: `max_concurrency` limits *in-flight* requests, the idle pool limits *retained sockets*. A hyper HTTP/1 connection's read buffer grows to service a large body, and an idle pooled connection keeps that buffer for `pool_idle_timeout` (90s). Measured at 500 RPS against the 8 MB `/big` route: `max_concurrency` 8 -> 43 MB, 32 -> 60 MB, 64 -> 138 MB, 1000 (the default) -> **821 MB** at 30s. The same 1000 default against a tiny-body route is 7.3 MB, so the buffers only inflate when responses are large — which is precisely a load generator's workload. Ceiling ~= `pool_max_idle_per_host` x per-connection read buffer (~400-800 KB). Memory is still flat in *request count* (703,896 requests -> 21.6 MB), so the ring/body bounds hold; the defect is per-*connection*. Fix is to decouple the two knobs and default the idle pool low, which is a *measurement* trade-off (a smaller pool means more handshakes) and must be documented as one.
- **HAZARD — the memory regression tests are `#[ignore]`d and CI runs them explicitly.** `memory_bound_test` and `concurrency_memory_test` allocate hundreds of MB against the 8 MB `/big` route, and `memory_bound_test`'s positive control deliberately retains 30 × 8 MB to prove the RSS probe can see retention. An ordinary `cargo test` therefore ran a load generator on the developer's machine, and that has taken down a desktop session three times. They are now opt-in: `cargo test --test memory_bound_test -- --ignored`. CI runs both on a disposable runner. **Do not remove the `#[ignore]`** — it is the only thing making `cargo test` safe on a 15 GB host, and re-enabling them locally is a decision for the person at the keyboard, not for an agent.
- **KNOWN GAP (P2) — `max_concurrency` has no CLI flag.** It is config-file only, while every other bound in `Config` is exposed on the command line. `--help` does not mention it.
- **KNOWN GAP — TUI does not run a load:** `src/cli/commands/tui_mode.rs` constructs a `LoadEngine` but never starts it, so the dashboard's `Ctrl+R` shows "RUNNING" while no traffic is generated. The interactive dashboard is currently a simulation. This is a feature-completion gap, not a memory-safety one, and is not yet fixed.
- **First engine-driving integration suite (8 new files under `tests/`, +43 integration tests, +28 unit tests):** `memory_bound_test` is the OOM regression guard and asserts a *scaling* property — two runs against `/big` at identical concurrency and different request counts must cost the same resident memory — plus a positive control that deliberately retains 30 × 8 MB bodies to prove the `/proc` RSS probe can actually see retention. `crash_safety_test` pins the capture cap at its exact boundary (2048 bytes = complete, 2049 = truncated), the 100k/16 retention ring, and the 64-key error budget. `throughput_bytes_test` proves bytes are counted from the stream by contrasting a real `content-length` route against a genuinely chunked in-test route. `scheduling_test` pins open-loop rate accuracy, saturation honesty, the concurrency ceiling high-water mark, and cancellation drain. `latency_percentile_test` is a regression guard for the quantile-scale bug against a deterministic 5%-at-250ms distribution. `config_test`, `report_test`, `error_path_test`, `structural_safety_test` cover validation-at-construction, config-file precedence, report round-trips, real transport failures over a raw `TcpListener`, and the terminal/mock/bound structural invariants. **Seven mutations were seeded and reverted to prove these tests go red** (percentile scale, `content_length` over stream, unbounded ring, dropped `validate()` call, config-file erasure, error-key budget, always-truncate).
- **Benchmarks (`benches/hot_path_bench.rs`, `benches/aggregation_bench.rs`, `harness = false`):** per-stage decomposition of the request path and the aggregation path, with hard allocation ceilings rather than latency thresholds. Both refuse to run under `debug_assertions`, because `cargo test --all-targets` includes bench targets and a `harness = false` bench's `main` *is* the test entry point — a benchmark that printed debug numbers would be evidence-shaped and meaningless.
- **Production seams added for testability (3 changes, all in service of a real test):** `DummyServer::router()` exposes the route table so a caller can bind an ephemeral port and read back the real address — `run()` takes a port and reports none, which made port-0 impossible; `/sized?bytes=N` serves an exactly-sized 500 body (the capture cap is a boundary, and only an exact-size fixture can prove the off-by-one); `executor::TRUNCATION_MARKER` is now `pub` so a report consumer can detect truncation from the constant rather than a drifting literal. `/sized` takes a **query** parameter, not a path parameter — see the finding below.
- **DEFECT (P1, unfixed) — a response that errors mid-body is reported as a success.** `src/runner/executor.rs:62` derives `success` from the status code alone and discards `outcome.error`, so a `200` whose body stream fails is counted in `success`, *not* in `fail`, while its error message is filed under `error_counts`. A run can report 100% success against a target returning unusable responses and no counter disagrees. `tests/error_path_test.rs::a_truncated_response_is_counted_as_a_failure_not_a_success` fails deliberately; the assertion must not be weakened.
- **DEFECT (P2, unfixed) — `max_concurrency` is inert in Users mode.** `runner/engine.rs` acquires a semaphore permit only in `run_rps`; `run_users` bounds concurrency by `num_users` alone. `Config::max_concurrency` is documented as "max concurrent in-flight requests (prevents OOM/Task explosion)", so a user setting it in Users mode gets no ceiling they asked for. Bounded by `MAX_ALLOWED_USERS`, so this is a semantic gap, not a memory-safety one. No test asserts the current behaviour, deliberately: enshrining it would make the bug look intended.
- **DEFECT (P3, unfixed) — a per-request `minijinja::Environment::new()` on the hot path.** `templates/engine.rs::execute_str` constructs an `Environment`, registers six functions and re-parses the template on *every* call, and it is the path the engine takes per request for a templated URL, body or header. This directly contradicts backend-style-guide §4 ("no template engine construction per request"). `PreparedRequest::new` pre-decomposes the request but `execute` is a convenience method that recompiles, so the pre-parsing buys nothing on the templated path. `benches/hot_path_bench.rs` reports the per-op allocation count and prints this as an open finding; its allocation ceiling is deliberately loose because the current cost is the defect.
- **DEFECT (P3, characterisation only) — empty inline body and empty `@file` body disagree.** `PreparedRequest::new` maps `Some("")` to "no body" (no `content-type` injected) but an empty `@file` to `Some("")` — a body that carries a `content-type`. `tests`-adjacent unit test `an_empty_body_file_is_still_treated_as_carrying_a_body` records the behaviour and labels it characterisation, not endorsement.
- **ENVIRONMENT (host, not code) — the local crates.io cache holds a `matchit-0.7.3` whose source implements the 0.6-era `:name` route syntax.** Every axum `{param}` route 404s on this machine, including when the router is called directly through `tower` with no socket involved. Verified by bisection: `/t/:n` matches, `/t/{n}` does not. This is why `/sized` takes a query parameter. Any future axum route in this crate using `{param}` will appear broken here and work on a clean registry — re-verify the cache before debugging the route.

