use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Result of a single request execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentResult {
    /// Scheduled time for this request.
    pub timestamp: DateTime<Utc>,

    /// Total latency from scheduled time to response completion.
    #[serde(with = "duration_serde")]
    pub latency: Duration,

    /// Service time from actual request start to response.
    #[serde(with = "duration_serde")]
    pub service_time: Duration,

    /// Queue wait time (scheduled vs actual start).
    #[serde(with = "duration_serde")]
    pub queue_wait: Duration,

    /// HTTP status code (or script exit code).
    pub status: u16,

    /// Whether the request was successful.
    pub success: bool,

    /// Response body size in bytes.
    pub bytes: i64,

    /// Virtual user ID.
    pub user_id: String,

    /// Query label (for JMeter CSV compatibility).
    #[serde(default = "default_query")]
    pub query: String,

    /// Error message (if any).
    pub error: Option<String>,

    /// Response body (captured for errors and HTTP >= 400).
    pub response_body: Option<String>,
}

fn default_query() -> String {
    "custom".to_string()
}

/// Serde serialization for Duration as microseconds (u64).
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(dur: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        dur.as_micros().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let micros = u64::deserialize(deserializer)?;
        Ok(Duration::from_micros(micros))
    }
}
