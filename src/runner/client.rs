use std::time::Duration;

use reqwest::Client;

use crate::core::config::Config;
use crate::core::constants::{DEFAULT_TIMEOUT_SECS, MAX_IDLE_CONNS};

/// Build the HTTP client used for load generation.
///
/// Connection pool sizing is tied to the configured concurrency ceiling:
/// a pool smaller than the ceiling would serialise requests and report a
/// generator bottleneck as target latency, while a pool with no idle timeout
/// retains sockets for the life of the process.
pub fn build_client(cfg: &Config) -> anyhow::Result<Client> {
    let timeout = if cfg.timeout_secs > 0 {
        Duration::from_secs(cfg.timeout_secs)
    } else {
        Duration::from_secs(DEFAULT_TIMEOUT_SECS)
    };

    let pool = cfg.max_concurrency.max(1) as usize;

    Client::builder()
        .pool_max_idle_per_host(pool.min(MAX_IDLE_CONNS))
        .pool_idle_timeout(Some(Duration::from_secs(90)))
        .tcp_keepalive(Duration::from_secs(60))
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(10)))
        .danger_accept_invalid_certs(true)
        .gzip(true)
        .brotli(true)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))
}
