use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Load generation mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Open loop — target RPS with time-based scheduling.
    Rps,
    /// Closed loop — fixed number of virtual users looping.
    Users,
}

impl Default for Mode {
    fn default() -> Self {
        Self::Rps
    }
}

/// Load test configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Target URL.
    pub url: String,

    /// HTTP method (GET, POST, etc.).
    #[serde(default = "default_method")]
    pub method: String,

    /// Request body (plain text or @file.json to read from file).
    pub body: Option<String>,

    /// HTTP headers.
    #[serde(default)]
    pub headers: IndexMap<String, String>,

    /// Target requests per second (open loop mode).
    #[serde(default = "default_target_rps")]
    pub target_rps: u32,

    /// Steady state duration in seconds.
    #[serde(default = "default_duration")]
    pub steady_dur_secs: u64,

    /// Ramp-up duration in seconds.
    #[serde(default)]
    pub ramp_up_secs: u64,

    /// Ramp-down duration in seconds.
    #[serde(default)]
    pub ramp_down_secs: u64,

    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Load generation mode.
    #[serde(default)]
    pub mode: Mode,

    /// Number of virtual users (closed loop mode).
    #[serde(default = "default_num_users")]
    pub num_users: u32,

    /// Think time between requests in milliseconds (closed loop mode).
    #[serde(default)]
    pub think_time_ms: u64,

    /// Shell command to execute (script mode).
    pub command: Option<String>,

    /// Output file prefix for reports.
    pub out_prefix: Option<String>,

    /// Max concurrent in-flight requests (prevents OOM/Task explosion).
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: default_method(),
            body: None,
            headers: IndexMap::new(),
            target_rps: default_target_rps(),
            steady_dur_secs: default_duration(),
            ramp_up_secs: 0,
            ramp_down_secs: 0,
            timeout_secs: default_timeout(),
            mode: Mode::default(),
            num_users: default_num_users(),
            think_time_ms: 0,
            command: None,
            out_prefix: None,
            max_concurrency: default_max_concurrency(),
        }
    }
}

impl Config {
    /// Total test duration.
    pub fn total_duration(&self) -> Duration {
        Duration::from_secs(self.ramp_up_secs + self.steady_dur_secs + self.ramp_down_secs)
    }

    /// Validate configuration.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.mode == Mode::Rps && self.target_rps == 0 {
            errors.push("target_rps must be greater than 0 in RPS mode".into());
        }

        if self.mode == Mode::Users && self.num_users == 0 {
            errors.push("num_users must be greater than 0 in Users mode".into());
        }

        if self.steady_dur_secs == 0 {
            errors.push("steady_dur_secs must be greater than 0".into());
        }

        if self.url.is_empty() && self.command.is_none() {
            errors.push("either url or command must be specified".into());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn default_method() -> String {
    "GET".to_string()
}

fn default_target_rps() -> u32 {
    100
}

fn default_duration() -> u64 {
    30
}

fn default_timeout() -> u64 {
    30
}

fn default_num_users() -> u32 {
    10
}

fn default_max_concurrency() -> u32 {
    1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.method, "GET");
        assert_eq!(cfg.target_rps, 100);
        assert_eq!(cfg.steady_dur_secs, 30);
        assert_eq!(cfg.mode, Mode::Rps);
    }

    #[test]
    fn test_total_duration() {
        let cfg = Config {
            ramp_up_secs: 5,
            steady_dur_secs: 30,
            ramp_down_secs: 5,
            ..Default::default()
        };
        assert_eq!(cfg.total_duration(), Duration::from_secs(40));
    }

    #[test]
    fn test_validate_empty_url_and_command() {
        let cfg = Config::default();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rps_zero() {
        let cfg = Config {
            url: "http://localhost".into(),
            target_rps: 0,
            mode: Mode::Rps,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_users_zero() {
        let cfg = Config {
            command: Some("echo test".into()),
            num_users: 0,
            mode: Mode::Users,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_ok() {
        let cfg = Config {
            url: "http://localhost:8080".into(),
            target_rps: 50,
            steady_dur_secs: 10,
            mode: Mode::Rps,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }
}
