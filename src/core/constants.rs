/// Application version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Application name.
pub const APP_NAME: &str = "rustress";

// ---------------------------------------------------------------------------
// Request-body capture limits
//
// A load generator points at servers it does not control. Without a hard cap
// on captured response bytes, a single 500 response carrying a multi-megabyte
// error page is buffered once per in-flight request, which is a trivial way to
// exhaust host memory. Capture is therefore bounded and truncation is reported.
// ---------------------------------------------------------------------------

/// Maximum bytes of a response body retained for error diagnostics.
pub const MAX_CAPTURED_BODY_BYTES: usize = 2_048;

/// Maximum bytes drained from a single response body before the rest is dropped.
pub const MAX_DRAINED_BODY_BYTES: u64 = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Result retention limits
//
// Per-request results are retained for report export only. Retention is a
// fixed-capacity ring buffer so that resident memory is a function of the
// configured capacity, never of the number of requests executed.
// ---------------------------------------------------------------------------

/// Number of per-request results retained in memory for report export.
pub const RESULT_RING_CAPACITY: usize = 50_000;

/// Maximum distinct error message keys tracked before folding into a bucket.
pub const MAX_TRACKED_ERROR_KEYS: usize = 64;

/// Label used for error keys beyond `MAX_TRACKED_ERROR_KEYS`.
pub const ERROR_KEY_OVERFLOW_LABEL: &str = "(other)";

// ---------------------------------------------------------------------------
// Scheduling limits
// ---------------------------------------------------------------------------

/// Lowest accepted value for `max_concurrency`. Zero would deadlock the engine.
pub const MIN_ALLOWED_CONCURRENCY: u32 = 1;

/// Highest accepted value for `max_concurrency`.
pub const MAX_ALLOWED_CONCURRENCY: u32 = 100_000;

/// Highest accepted value for `num_users` in closed-loop mode.
pub const MAX_ALLOWED_USERS: u32 = 100_000;

/// Highest accepted value for `target_rps` in open-loop mode.
pub const MAX_ALLOWED_RPS: u32 = 1_000_000;

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

/// Default HTTP method.
pub const DEFAULT_METHOD: &str = "GET";

/// Default request timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Default test duration in seconds.
pub const DEFAULT_DURATION_SECS: u64 = 30;

/// Default target RPS.
pub const DEFAULT_TARGET_RPS: u32 = 100;

/// Default number of virtual users.
pub const DEFAULT_NUM_USERS: u32 = 10;

/// Default think time in milliseconds.
pub const DEFAULT_THINK_TIME_MS: u64 = 1000;

/// Capacity of the stats-update channel between the engine and its consumer.
///
/// Bounded on purpose. An unbounded channel on this path is only safe while the
/// consumer keeps up, and nothing enforces that: a stalled consumer (a blocked
/// terminal, a suspended process) turns every tick into retained memory. Frames
/// are progress *hints* for the TUI and progress bar; the authoritative numbers
/// are read from the counters directly, so a dropped frame costs nothing. The
/// consumer drains all pending frames and keeps the newest, so a full channel
/// simply means the consumer is behind and the next tick supersedes this one.
pub const STATS_CHANNEL_CAPACITY: usize = 64;

/// Default ceiling on concurrently executing requests.
///
/// **This is the knob that sets peak memory on large responses.** Each
/// in-flight request holds a hyper HTTP/1 read buffer grown to service the
/// body it is reading, and reqwest 0.12 exposes no `http1_max_buf_size` knob,
/// so in-flight count is the only lever available. Measured at 500 RPS against
/// 8 MB bodies, varying this with the idle pool pinned at 1000 so the two
/// bounds could not be confused:
///
/// | max_concurrency | peak RSS | marginal per request |
/// |---|---|---|
/// | 8    | 53 MB  | |
/// | 32   | 77 MB  | |
/// | 64   | 121 MB | |
/// | 256  | 254 MB | |
/// | 1000 | 750 MB | ~750 KB |
///
/// The relationship is linear and only applies to large bodies: the same 1000
/// in-flight run against a tiny-body route peaked at 7.3 MB, because the
/// buffer never grows past what a small response needs. The cost appears
/// exactly when a load generator is doing its job.
///
/// The previous default of 1000 therefore permitted ~750 MB of resident
/// memory with no warning, which is the OOM this project exists to prevent.
/// 128 caps the worst case near 96 MB and is raised explicitly, with
/// `--max-concurrency`, by anyone whose target needs it.
///
/// Raising it is a real trade-off, not a free win: a slower target needs more
/// in-flight requests to sustain a given RPS, so a low ceiling makes the
/// generator the bottleneck. That surfaces honestly as `dropped_scheduled`,
/// which marks the run's latency figures invalid, rather than as silent
/// queueing.
pub const DEFAULT_MAX_CONCURRENCY: u32 = 128;

// ---------------------------------------------------------------------------
// Connection pool
// ---------------------------------------------------------------------------

/// Max idle connections per host.
pub const MAX_IDLE_CONNS_PER_HOST: usize = 2_000;

/// Max total idle connections.
pub const MAX_IDLE_CONNS: usize = 2_000;

/// Max connections per host.
pub const MAX_CONNS_PER_HOST: usize = 2_000;

/// Default idle connections retained per host.
///
/// Independent of `max_concurrency`: the first bounds requests *executing*, the
/// second bounds sockets *retained*. Deriving one from the other made a run
/// that peaked at 1000 in-flight retain 1000 sockets, which is wasteful but
/// was measured to be a *secondary* term, not the dominant one. See
/// `DEFAULT_MAX_CONCURRENCY` for the term that actually sets the ceiling.
///
/// 64 keeps enough sockets warm for a steady-state run to measure connection
/// reuse rather than handshakes, which is the thing a load generator exists to
/// measure. Raising it trades memory for handshake-free latency figures.
pub const DEFAULT_POOL_MAX_IDLE_PER_HOST: usize = 64;

/// Default seconds an unused connection is kept warm.
///
/// The previous 90s outlived most of a test run, so a burst's sockets stayed
/// resident long past the burst. Short enough to release them, long enough to
/// hold connections warm across ordinary inter-request gaps.
pub const DEFAULT_POOL_IDLE_TIMEOUT_SECS: u64 = 15;

/// Upper bound on `pool_idle_timeout_secs`. Retaining sockets for minutes
/// defeats the purpose of bounding them.
pub const MAX_POOL_IDLE_TIMEOUT_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// Telemetry cadence
// ---------------------------------------------------------------------------

/// Stats update interval in milliseconds.
pub const STATS_UPDATE_INTERVAL_MS: u64 = 100;

/// Headless progress bar update interval in milliseconds.
pub const PROGRESS_UPDATE_INTERVAL_MS: u64 = 200;

// ---------------------------------------------------------------------------
// Histogram bounds
// ---------------------------------------------------------------------------

/// Histogram range: 1 microsecond to 10 minutes, 3 significant figures.
pub const HISTOGRAM_LOW_US: u64 = 1;
pub const HISTOGRAM_HIGH_US: u64 = 600_000_000;
pub const HISTOGRAM_SIGFIGS: u8 = 3;
