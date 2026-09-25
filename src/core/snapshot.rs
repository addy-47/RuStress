use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Point-in-time snapshot of stats, sent to the TUI over a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsSnapshot {
    pub requests: u64,
    pub success: u64,
    pub fail: u64,
    pub bytes: u64,
    pub inflight: i64,

    /// Pre-calculated percentiles (microseconds).
    pub p50_service_ms: f64,
    pub p90_service_ms: f64,
    pub p95_service_ms: f64,
    pub p99_service_ms: f64,
    pub max_service_ms: f64,
    pub mean_service_ms: f64,

    /// Average queue wait (milliseconds).
    pub avg_queue_wait_ms: f64,

    /// Status code distribution.
    pub status_codes: IndexMap<u16, u64>,

    /// Error message counts.
    pub error_counts: IndexMap<String, u64>,

    /// Sample response bodies by status code.
    pub response_samples: IndexMap<u16, String>,
}

impl Default for StatsSnapshot {
    fn default() -> Self {
        Self {
            requests: 0,
            success: 0,
            fail: 0,
            bytes: 0,
            inflight: 0,
            p50_service_ms: 0.0,
            p90_service_ms: 0.0,
            p95_service_ms: 0.0,
            p99_service_ms: 0.0,
            max_service_ms: 0.0,
            mean_service_ms: 0.0,
            avg_queue_wait_ms: 0.0,
            status_codes: IndexMap::new(),
            error_counts: IndexMap::new(),
            response_samples: IndexMap::new(),
        }
    }
}
