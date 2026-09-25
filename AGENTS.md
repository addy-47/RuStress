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
   cargo clippy --all-targets -- -D warnings
   cargo test --all-targets
   cargo test --doc
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

### 4.4 HARD GATE: Code Modification Gate

> 🛑 **Before ANY write task — production, test, or bench — read the matching files in `.agents/rules/`.**

| Working in | Read |
|---|---|
| `src/` production code | `backend-style-guide.md` + `backend-engineer.md` |
| `tests/`, `benches/`, `examples/` | `testing-style-guilde.md` + `test-engineer.md` |
| Reviewing or gating | `qa-engineer.md` |
| New features, module boundaries, dependencies, public API | `system-architect.md` |
| `/` `frontend-engineer.md` — **not applicable.** This project has no web frontend. The TUI is Rust. |

---

## 5. Recent Work

- **Collapse to a single crate:** 8 workspace crates (`rustress-core`, `-runner`, `-tui`, `-metrics`, `-templates`, `-export`, `-dummy`, `-cli`) merged into one `Cargo.toml` publishing a lib + bin. Import paths rewritten from `rustress_*::` to `crate::<domain>::`; sibling imports to `super::`.
- **P0 memory bounds (crash fix):** unbounded `Vec<ExperimentResult>` → fixed-capacity `ResultLog` ring (50k, evictions counted); unbounded `IndexMap<String, u64>` error map → 64-key budget + overflow bucket; response body capture capped at 2 KB with an explicit truncation marker; bodies streamed with `Response::chunk()` instead of `bytes()`/`text()`, so bytes are counted for chunked responses.
- **P0 unbounded task spawn:** `run_rps` accumulated a `JoinHandle` per request forever. The concurrency permit is now acquired *before* spawning via `try_acquire_owned`; on saturation the request is dropped and counted in `dropped_scheduled`. Added `TUI render_shed_notice` so a saturated generator is visible, not silent.
- **P0 terminal safety:** added `tui::TerminalGuard` (RAII) owning raw mode, alternate screen, and mouse capture. Removed `panic = "abort"` from the release profile so `Drop` actually runs on panic.
- **P1 hot path:** introduced `PreparedRequest` so method parsing, header validation, `content-type` detection, and `@file` body loading happen once at construction instead of per request. Eliminates a per-request `minijinja::Environment` build, a per-header `to_lowercase()` allocation, and a per-request blocking file read. A missing `@file` now fails at startup instead of silently sending empty bodies.
- **P1 config validation:** rejects `max_concurrency = 0` (which deadlocked the engine), out-of-range `max_concurrency` / `num_users` / `target_rps`, `timeout_secs = 0`, and relative URLs.
- **Packaging:** added `rustfmt.toml`, crates.io metadata, `exclude`, and `panic = "abort"` removal; dropped the pre-existing `unused variable` warning; `cargo clippy --all-targets` is clean.
