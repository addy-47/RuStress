# Changelog

All notable changes to RuStress are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1]

> Post-release note: the memory assertion in `concurrency_memory_test` as
> tagged could not hold in CI, and three successive attempts to make it hold
> exposed why it never should have been written that way. It tried to measure
> peak RSS from inside the test process, where the zero-mock rule requires the
> 8 MB-serving target to run too — so on CI the *higher* in-flight ceiling
> measured *lower* (101820 KB vs 93284 KB), the server swamping the generator.
> Earlier drafts read `VmRSS` at one instant, and read the monotonic `VmHWM` with
> a `clear_refs` reset whose error was discarded. All passed locally, all failed
> on CI. The relationship is real but only observable out-of-process.
>
> Fixed on `master` by dropping the relationship assertion and keeping the
> deterministic part — the shipped default must imply a worst case inside a sane
> budget — which allocates nothing and is mutation-validated. The measured table
> is reproduced by `cargo run --release --example bounded_memory`. The 0.1.1
> crate is unaffected: the defect was in the test's measurement, not the
> product.

Memory, hot path, and the interactive dashboard.

### Fixed

- **Peak memory was ~750 MB on large responses, and the ceiling was
  `max_concurrency`.** Each in-flight request holds a hyper HTTP/1 read buffer
  grown to service the body it is reading, and reqwest 0.12 exposes no
  `http1_max_buf_size` knob, so the in-flight count was the only lever. Measured
  at 500 RPS against 8 MB bodies, with the connection pool held constant:

  | `max_concurrency` | peak RSS |
  |---|---|
  | 8 | 53 MB |
  | 64 | 121 MB |
  | 256 | 254 MB |
  | 1000 | 750 MB |

  The default of 1000 therefore permitted three quarters of a gigabyte of
  resident memory with no warning. The default is now 128, which caps the worst
  case near 96 MB. The same 1000 in-flight run against a *small*-body route
  peaks at 7.3 MB — the cost appears exactly when a load generator is doing its
  job.

- **The interactive TUI never generated any load.** It constructed a
  `LoadEngine` and never called `run`, so `Ctrl+R` displayed "RUNNING" over a
  dashboard whose counters stayed at zero. Run lifecycle now lives in
  `runner::RunController`, which is testable without a TTY, and the TUI drives
  it.

- **A script's output was buffered without bound.** `Command::output()` collects
  both streams to EOF, so a script printing without limit exhausted host memory.
  stdout is now streamed and counted but never retained; stderr is capped at the
  same 2 KB limit as a response body and marked when truncated.

- **The stats channel was unbounded.** A stalled consumer turned every 10 Hz
  frame into retained memory. It is now bounded, with `try_send`: a dropped
  frame is a display hint and costs no measurement, because the final summary
  reads the counters directly.

- **Templates were re-parsed on every request.** `TemplateEngine::execute_str`
  built a `minijinja::Environment`, registered six functions and re-parsed the
  template per call — on the path the executor takes for every templated URL,
  header and body. One environment is now built once and each distinct template
  is compiled into it exactly once. The old `templates` cache was write-only and
  was never read.

### Added

- `--max-concurrency`, `--pool-max-idle-per-host` and `--pool-idle-timeout`.
  `max_concurrency` was configurable only through a TOML file while every other
  bound in `Config` had a flag.
- `pool_max_idle_per_host` and `pool_idle_timeout_secs` in `Config`, with
  validation. `pool_max_idle_per_host` is no longer derived from
  `max_concurrency`; the two bound different things (requests executing vs
  sockets retained) and conflating them is what made the original diagnosis of
  the memory ceiling wrong. The idle timeout drops from 90 s to 15 s so a burst's
  sockets are not retained long past the burst.
- `runner::RunController`: build, spawn, cancel, drain and join for a load run,
  independent of any terminal.
- `tests/tui_run_test.rs` — proves starting a run produces real traffic, that a
  stop drains and releases every in-flight request, and that a second start is
  refused rather than orphaning the first run's task.
- `tests/concurrency_memory_test.rs` — pins that peak memory tracks the
  in-flight ceiling, and that the shipped default cannot burst back toward the
  old 750 MB worst case. Mutation-validated.
- `examples/` — six runnable programs over the public API: `minimal_rps`,
  `closed_loop_users`, `templated_payload`, `file_body`, `custom_endpoint`,
  `bounded_memory`. CI builds them; an example that no longer compiles is dead
  documentation.
- The dummy target answers POST as well as GET. A GET-only fixture answers a
  question about a target's write path with a 405, which measures the method
  check rather than the handler.
- `CHANGELOG.md`, and `cargo build --examples` in CI.
- The memory regression tests are now `#[ignore]`d, with CI running them
  explicitly. They allocate hundreds of MB against an 8 MB route, and an
  ordinary `cargo test` had become a memory hazard on the machine they were
  written on — it has taken down a desktop session twice. Opt-in locally, always
  run in CI:
  `cargo test --test memory_bound_test -- --ignored`

### Changed

- The `max_concurrency` documentation now states that it is an open-loop (RPS)
  bound. Closed-loop mode is bounded by `num_users` alone, deliberately: a
  virtual user *is* the unit of concurrency there, and a second independent
  control would make the mode's throughput unexplainable. The previous wording
  ("prevents OOM/Task explosion") implied it applied to both modes, and it did
  not.
- `EventLoop` no longer carries the stats channel. That responsibility belongs
  to `RunController`, and routing every measurement through the TUI event loop
  made it untestable.
- The banner version is derived from `CARGO_PKG_VERSION` instead of a literal,
  so it cannot drift from the manifest.

### Known limitations

- Peak memory is still a function of `max_concurrency` and the target's response
  size, and cannot be reduced further through reqwest. Roughly 750 KB per
  in-flight request on large bodies; negligible on small ones. Raise the ceiling
  deliberately and watch the figure.
- `danger_accept_invalid_certs(true)` remains unconditional. It is deliberate for
  a staging target with a self-signed certificate, and it means the tool cannot
  measure TLS handshake rejection. It is not yet a flag.
- A run that sheds requests reports `dropped_scheduled` and marks its latency
  figures invalid rather than queueing, but the ceiling that causes the shedding
  is not surfaced as a recommendation.

## [0.1.0]

First published release.

- Single-crate layout publishing a library (`rustress`) and a binary.
- Bounded result retention, capped response-body capture, bounded error key
  space, and permits acquired before spawning.
- `tui::TerminalGuard` for terminal restoration; `panic = "abort"` removed from
  every profile.
- Corrected latency percentiles, which had all been reporting the maximum
  observed latency.
- Headless and TUI summaries derived from authoritative counters.
- CI, and `scripts/safe-cargo.sh` for memory-capped local builds.
