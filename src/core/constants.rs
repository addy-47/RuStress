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

/// Default ceiling on concurrently executing requests.
pub const DEFAULT_MAX_CONCURRENCY: u32 = 1_000;

// ---------------------------------------------------------------------------
// Connection pool
// ---------------------------------------------------------------------------

/// Max idle connections per host.
pub const MAX_IDLE_CONNS_PER_HOST: usize = 2_000;

/// Max total idle connections.
pub const MAX_IDLE_CONNS: usize = 2_000;

/// Max connections per host.
pub const MAX_CONNS_PER_HOST: usize = 2_000;

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
