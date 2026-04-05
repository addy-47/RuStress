/// Application version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Application name.
pub const APP_NAME: &str = "rustress";

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

/// Max idle connections per host.
pub const MAX_IDLE_CONNS_PER_HOST: usize = 2000;

/// Max total idle connections.
pub const MAX_IDLE_CONNS: usize = 2000;

/// Max connections per host.
pub const MAX_CONNS_PER_HOST: usize = 2000;

/// Stats update interval in milliseconds.
pub const STATS_UPDATE_INTERVAL_MS: u64 = 100;

/// Headless progress bar update interval in milliseconds.
pub const PROGRESS_UPDATE_INTERVAL_MS: u64 = 200;

/// Histogram range: 1 microsecond to 10 minutes, 3 significant figures.
pub const HISTOGRAM_LOW_US: u64 = 1;
pub const HISTOGRAM_HIGH_US: u64 = 600_000_000; // 10 minutes in microseconds
pub const HISTOGRAM_SIGFIGS: u8 = 3;
