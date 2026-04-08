use reqwest::Client;
use rustress_core::config::Config;
use rustress_core::constants::MAX_IDLE_CONNS_PER_HOST;

/// Build an optimized HTTP client for load testing.
pub fn build_client(cfg: &Config) -> Client {
    let timeout = if cfg.timeout_secs > 0 {
        std::time::Duration::from_secs(cfg.timeout_secs)
    } else {
        std::time::Duration::from_secs(30)
    };

    Client::builder()
        .pool_max_idle_per_host(MAX_IDLE_CONNS_PER_HOST)
        .pool_idle_timeout(None)
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .timeout(timeout)
        .danger_accept_invalid_certs(true)
        .gzip(true)
        .brotli(true)
        .build()
        .expect("valid HTTP client configuration")
}
