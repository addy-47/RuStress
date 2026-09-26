use std::time::Duration;

use reqwest::Client;

use crate::core::config::Config;
use crate::core::constants::{DEFAULT_TIMEOUT_SECS, MAX_IDLE_CONNS_PER_HOST};

/// Build the HTTP client used for load generation.
///
/// # Connection pool sizing
///
/// `pool_max_idle_per_host` and `max_concurrency` bound **different things**
/// and are deliberately not derived from one another any more:
///
/// * `max_concurrency` — requests *executing* right now. Work, not memory.
/// * `pool_max_idle_per_host` — sockets *retained after* finishing. Memory.
///
/// They were previously conflated, so the idle pool inherited the in-flight
/// ceiling. That is wasteful, but measurement shows it is *not* what sets the
/// memory ceiling: holding this at 1000 and cutting the idle pool from 1000 to
/// 64 changed peak RSS by 0.6%. The term that actually dominates is
/// `max_concurrency`, because every in-flight request holds a hyper read buffer
/// sized to its body and reqwest exposes no knob to shrink it. See
/// `DEFAULT_MAX_CONCURRENCY` for that measurement.
///
/// Both are kept configurable because they pull in opposite directions for the
/// *measurement*, not just for memory. A larger idle pool means fewer TCP and
/// TLS handshakes, so a run measures steady-state throughput. A smaller one
/// means handshakes appear in the latency figures.
///
/// # TLS
///
/// Certificate verification is **disabled unconditionally**. That is the right
/// default for a load generator pointed at staging targets with self-signed or
/// expired certificates, and it is why the tool can measure a service whose
/// TLS a real client would reject. The consequence to be aware of: the tool
/// cannot measure TLS handshake rejection, and a target with an expired
/// certificate reports 200s here. Expose this as a flag rather than leaving it
/// implicit when adding a TLS-sensitive measurement mode.
pub fn build_client(cfg: &Config) -> anyhow::Result<Client> {
    let timeout = if cfg.timeout_secs > 0 {
        Duration::from_secs(cfg.timeout_secs)
    } else {
        Duration::from_secs(DEFAULT_TIMEOUT_SECS)
    };

    let idle_per_host = cfg.pool_max_idle_per_host.max(1) as usize;
    let idle_timeout = Duration::from_secs(cfg.pool_idle_timeout_secs);

    Client::builder()
        .pool_max_idle_per_host(idle_per_host.min(MAX_IDLE_CONNS_PER_HOST))
        .pool_idle_timeout(Some(idle_timeout))
        .tcp_keepalive(Duration::from_secs(60))
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(10)))
        .danger_accept_invalid_certs(true)
        .gzip(true)
        .brotli(true)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))
}
