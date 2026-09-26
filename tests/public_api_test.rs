//! ============================================================================
//! public_api_test — public crate surface contracts
//! ============================================================================
//! Category     : Integration Test
//! Component    : crate root (`rustress::*`)
//! Prerequisites: none (no network, no fixtures)
//! Execution    : cargo test --test public_api_test
//! Metrics      : pass/fail over the published public API surface
//! ============================================================================

use rustress::core::config::{Config, Mode};

#[test]
fn version_is_a_semver_string() {
    let version = rustress::core::constants::VERSION;
    assert!(!version.is_empty());
    assert!(
        version.split('.').count() >= 2,
        "version must be semver-shaped, got {version}"
    );
}

#[test]
fn config_round_trips_through_serde() {
    let cfg = Config {
        url: "http://test".into(),
        mode: Mode::Users,
        num_users: 50,
        steady_dur_secs: 10,
        ..Default::default()
    };

    let json = serde_json::to_string(&cfg).unwrap();
    let parsed: Config = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.mode, Mode::Users);
    assert_eq!(parsed.num_users, 50);
    assert_eq!(parsed.url, "http://test");
}

#[test]
fn experiment_result_round_trips_through_serde() {
    use chrono::Utc;
    use rustress::core::result::ExperimentResult;
    use std::time::Duration;

    let original = ExperimentResult {
        timestamp: Utc::now(),
        latency: Duration::from_millis(100),
        service_time: Duration::from_millis(90),
        queue_wait: Duration::from_millis(10),
        status: 503,
        success: false,
        bytes: 2048,
        user_id: "user-1".into(),
        query: "test".into(),
        error: Some("upstream unavailable".into()),
        response_body: Some("body".into()),
    };

    let decoded: ExperimentResult =
        serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();

    assert_eq!(decoded.status, 503);
    assert!(!decoded.success);
    assert_eq!(decoded.latency, original.latency);
    assert_eq!(decoded.service_time, original.service_time);
    assert_eq!(decoded.queue_wait, original.queue_wait);
    assert_eq!(decoded.error.as_deref(), Some("upstream unavailable"));
}

#[test]
fn config_file_shape_deserializes_from_toml() {
    let toml = r#"
        url = "http://127.0.0.1:8080/fast"
        method = "POST"
        target_rps = 250
        steady_dur_secs = 15
        ramp_up_secs = 5
        mode = "rps"
        max_concurrency = 64

        [headers]
        Authorization = "Bearer token"
    "#;

    let cfg: Config = toml::from_str(toml).unwrap();

    assert_eq!(cfg.url, "http://127.0.0.1:8080/fast");
    assert_eq!(cfg.method, "POST");
    assert_eq!(cfg.target_rps, 250);
    assert_eq!(cfg.ramp_up_secs, 5);
    assert_eq!(cfg.max_concurrency, 64);
    assert_eq!(
        cfg.headers.get("Authorization").map(String::as_str),
        Some("Bearer token")
    );
    assert!(cfg.validate().is_ok(), "a well-formed file must validate");
}

#[test]
fn default_config_is_valid_enough_to_run() {
    let cfg = Config {
        url: "http://127.0.0.1:8080".into(),
        ..Default::default()
    };
    cfg.validate()
        .expect("default config with a url must validate");

    assert!(
        cfg.total_duration().as_secs() > 0,
        "a run must have a duration"
    );
}
