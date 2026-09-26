//! ============================================================================
//! config_test — validation is enforced, and the config file is not erased
//! ============================================================================
//! Category     : Integration Test
//! Component    : `runner::engine::LoadEngine::new`, `core::config::Config::validate`,
//!                `cli::args::Cli::into_config`
//! Prerequisites: none (real `tempfile` files on disk)
//! Execution    : cargo test --test config_test
//! Metrics      : accept/reject decision, rejection reasons, resolved field values
//! ============================================================================
//!
//! Two separate regressions live here.
//!
//! `Config::validate` once had no production caller, so every bound it enforced
//! was inert: `--users 100000000` reached `run_users` unbounded. Validation now
//! runs inside `LoadEngine::new`, and these tests assert the rejection happens
//! there — which also means a library embedder cannot bypass it.
//!
//! `body` and `out_prefix` were assigned unconditionally from `Option`s, so
//! `--config load.toml` sent bodyless requests and exported nothing while
//! reporting a clean run.

mod common;

use clap::Parser;
use indexmap::IndexMap;
use std::path::Path;

use rustress::cli::args::Cli;
use rustress::core::config::{Config, Mode};
use rustress::core::constants::{
    MAX_ALLOWED_CONCURRENCY, MAX_ALLOWED_RPS, MAX_ALLOWED_USERS, MIN_ALLOWED_CONCURRENCY,
    STATS_CHANNEL_CAPACITY,
};
use rustress::runner::LoadEngine;
use tokio::sync::mpsc;

use common::cfg_for;

/// Assert `LoadEngine::new` rejects `cfg`, and return the combined reason text.
///
/// The check is on the constructor's return value, not on a later run: a bound
/// enforced anywhere other than construction can be bypassed by an embedder.
fn rejection_reason(cfg: Config) -> String {
    let (tx, _rx) = mpsc::channel(STATS_CHANNEL_CAPACITY);
    match LoadEngine::new(cfg, tx) {
        Ok(_) => panic!("LoadEngine::new accepted a configuration it must reject"),
        Err(e) => e.to_string(),
    }
}

/// A zero-permit semaphore deadlocks the engine: every request blocks forever
/// and the run never terminates.
#[test]
fn zero_concurrency_is_rejected_at_construction() {
    let mut cfg = cfg_for("http://127.0.0.1:1/x");
    cfg.max_concurrency = MIN_ALLOWED_CONCURRENCY - 1;
    let reason = rejection_reason(cfg);
    assert!(
        reason.contains("max_concurrency"),
        "the reason must name the offending field: {reason}"
    );
}

/// A concurrency ceiling above the allowed maximum would let one run allocate
/// an unbounded number of sockets and tasks.
#[test]
fn concurrency_above_the_ceiling_is_rejected_at_construction() {
    let mut cfg = cfg_for("http://127.0.0.1:1/x");
    cfg.max_concurrency = MAX_ALLOWED_CONCURRENCY + 1;
    let reason = rejection_reason(cfg);
    assert!(reason.contains("max_concurrency"), "{reason}");
}

/// Open loop with a zero rate never dispatches, so the run reports a clean
/// pass having measured nothing.
#[test]
fn zero_target_rps_is_rejected_at_construction() {
    let mut cfg = cfg_for("http://127.0.0.1:1/x");
    cfg.target_rps = 0;
    let reason = rejection_reason(cfg);
    assert!(reason.contains("target_rps"), "{reason}");
}

/// The rate ceiling exists because the schedule is a per-request slot; above it
/// the period underflows the timer and the run degenerates.
#[test]
fn target_rps_above_the_ceiling_is_rejected_at_construction() {
    let mut cfg = cfg_for("http://127.0.0.1:1/x");
    cfg.target_rps = MAX_ALLOWED_RPS + 1;
    let reason = rejection_reason(cfg);
    assert!(reason.contains("target_rps"), "{reason}");
}

/// Closed loop with zero users spawns nothing and reports a clean pass.
#[test]
fn zero_users_is_rejected_at_construction() {
    let mut cfg = cfg_for("http://127.0.0.1:1/x");
    cfg.mode = Mode::Users;
    cfg.num_users = 0;
    let reason = rejection_reason(cfg);
    assert!(reason.contains("num_users"), "{reason}");
}

/// The user ceiling is the only bound on closed-loop task count.
#[test]
fn users_above_the_ceiling_are_rejected_at_construction() {
    let mut cfg = cfg_for("http://127.0.0.1:1/x");
    cfg.mode = Mode::Users;
    cfg.num_users = MAX_ALLOWED_USERS + 1;
    let reason = rejection_reason(cfg);
    assert!(reason.contains("num_users"), "{reason}");
}

/// A zero timeout is applied as a request timeout of zero, which fails every
/// request instantly and reports the fastest possible target.
#[test]
fn zero_timeout_is_rejected_at_construction() {
    let mut cfg = cfg_for("http://127.0.0.1:1/x");
    cfg.timeout_secs = 0;
    let reason = rejection_reason(cfg);
    assert!(reason.contains("timeout_secs"), "{reason}");
}

/// A zero steady phase makes the schedule empty, so the run measures nothing
/// and reports success.
#[test]
fn zero_duration_is_rejected_at_construction() {
    let mut cfg = cfg_for("http://127.0.0.1:1/x");
    cfg.steady_dur_secs = 0;
    let reason = rejection_reason(cfg);
    assert!(reason.contains("steady_dur_secs"), "{reason}");
}

/// A relative URL reaches the client as a builder error on every request, so
/// the run reports a target failure for a configuration mistake.
#[test]
fn a_relative_url_is_rejected_at_construction() {
    let mut cfg = cfg_for("http://127.0.0.1:1/x");
    cfg.url = "127.0.0.1:8080/fast".into();
    let reason = rejection_reason(cfg);
    assert!(reason.contains("url"), "{reason}");
}

/// A validation error must be actionable, not just present.
///
/// A report naming one field at a time sends the user on a round trip per
/// attempt; the bounds are all known up front.
#[test]
fn every_violation_is_reported_not_just_the_first() {
    let cfg = Config {
        url: "not-a-url".into(),
        max_concurrency: 0,
        timeout_secs: 0,
        steady_dur_secs: 0,
        target_rps: 0,
        ..Default::default()
    };
    let reason = rejection_reason(cfg);

    for field in [
        "url",
        "max_concurrency",
        "timeout_secs",
        "steady_dur_secs",
        "target_rps",
    ] {
        assert!(
            reason.contains(field),
            "the reason must name {field} as well: {reason}"
        );
    }
}

/// A runnable configuration must be accepted.
///
/// Without this, every rejection above would also pass against a constructor
/// that rejects everything.
#[test]
fn a_valid_configuration_is_accepted() {
    let cfg = Config {
        url: "http://127.0.0.1:1/x".into(),
        target_rps: 25,
        steady_dur_secs: 1,
        timeout_secs: 5,
        max_concurrency: 8,
        headers: IndexMap::new(),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel(STATS_CHANNEL_CAPACITY);
    assert!(
        LoadEngine::new(cfg, tx).is_ok(),
        "a well-formed configuration must be accepted"
    );
}

/// The exact boundary values are accepted, not just values comfortably inside.
///
/// A ceiling that rejects its own maximum is a bound off by one, and an
/// operator who reads `MAX_ALLOWED_USERS` as "the most I may set" is misled.
#[test]
fn the_boundary_values_of_every_bound_are_accepted() {
    let cfg = Config {
        url: "http://127.0.0.1:1/x".into(),
        target_rps: MAX_ALLOWED_RPS,
        num_users: MAX_ALLOWED_USERS,
        max_concurrency: MAX_ALLOWED_CONCURRENCY,
        steady_dur_secs: 1,
        timeout_secs: 1,
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel(STATS_CHANNEL_CAPACITY);
    assert!(
        LoadEngine::new(cfg, tx).is_ok(),
        "each documented maximum must itself be accepted"
    );

    let cfg = Config {
        url: "http://127.0.0.1:1/x".into(),
        max_concurrency: MIN_ALLOWED_CONCURRENCY,
        steady_dur_secs: 1,
        timeout_secs: 1,
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel(STATS_CHANNEL_CAPACITY);
    assert!(
        LoadEngine::new(cfg, tx).is_ok(),
        "the documented minimum concurrency must be accepted"
    );
}

/// A config file's body and output prefix survive a CLI run that names neither.
///
/// Regression guard for the silent-erasure bug: `--config load.toml` used to
/// send bodyless requests and export nothing while reporting a clean run.
#[test]
fn config_file_body_and_out_prefix_survive_a_cli_run_that_passes_neither() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_load_profile(dir.path());

    let cli = Cli::parse_from(["rustress", "--config", path.to_str().unwrap()]);
    let cfg = cli.into_config();

    assert_eq!(
        cfg.body.as_deref(),
        Some(r#"{"id":"{{ uuid() }}"}"#),
        "a body declared only in the config file must reach the engine"
    );
    assert_eq!(
        cfg.out_prefix.as_deref(),
        Some("profile-run"),
        "an output prefix declared only in the config file must reach the engine"
    );
    assert_eq!(
        cfg.url, "http://127.0.0.1:9/profile",
        "the file's url must be kept when no --url is passed"
    );
    assert_eq!(cfg.target_rps, 250, "the file's rate must be kept");
    assert_eq!(cfg.steady_dur_secs, 15, "the file's duration must be kept");
    assert_eq!(cfg.max_concurrency, 64, "the file's ceiling must be kept");
    assert_eq!(cfg.method, "POST", "the file's method must be kept");
    assert_eq!(
        cfg.headers.get("Authorization").map(String::as_str),
        Some("Bearer from-file"),
        "the file's headers must be kept"
    );
    assert!(
        cfg.validate().is_ok(),
        "the resolved config must be runnable: {:?}",
        cfg.validate()
    );
}

/// An explicit flag still wins over the file.
///
/// The precedence fix must not have been a blanket "never override".
#[test]
fn an_explicit_flag_overrides_the_config_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_load_profile(dir.path());

    let cli = Cli::parse_from([
        "rustress",
        "--config",
        path.to_str().unwrap(),
        "--url",
        "http://127.0.0.1:9/override",
        "--body",
        "from-cli",
        "--out",
        "cli-run",
        "-r",
        "7",
        "-d",
        "3",
    ]);
    let cfg = cli.into_config();

    assert_eq!(cfg.url, "http://127.0.0.1:9/override");
    assert_eq!(cfg.body.as_deref(), Some("from-cli"));
    assert_eq!(cfg.out_prefix.as_deref(), Some("cli-run"));
    assert_eq!(cfg.target_rps, 7);
    assert_eq!(cfg.steady_dur_secs, 3);
    assert_eq!(
        cfg.method, "POST",
        "a field not passed on the command line must still come from the file"
    );
}

/// `--users` switches the mode, and a config file's mode is not lost otherwise.
#[test]
fn the_users_flag_selects_closed_loop() {
    let cli = Cli::parse_from(["rustress", "-u", "http://127.0.0.1:9/x", "-n", "12"]);
    let cfg = cli.into_config();
    assert_eq!(cfg.mode, Mode::Users);
    assert_eq!(cfg.num_users, 12);
}

/// A header given on the command line reaches the resolved config in the form
/// the executor will validate.
#[test]
fn command_line_headers_reach_the_resolved_config() {
    let cli = Cli::parse_from([
        "rustress",
        "-u",
        "http://127.0.0.1:9/x",
        "-H",
        "X-Trace: abc123",
        "-H",
        "X-Other:  spaced  ",
    ]);
    let cfg = cli.into_config();
    assert_eq!(
        cfg.headers.get("X-Trace").map(String::as_str),
        Some("abc123")
    );
    assert_eq!(
        cfg.headers.get("X-Other").map(String::as_str),
        Some("spaced"),
        "header values must be trimmed on both sides"
    );
}

/// Write a load profile to a real file in `dir` and return its path.
fn write_load_profile(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("load.toml");
    std::fs::write(
        &path,
        r#"
url = "http://127.0.0.1:9/profile"
method = "POST"
body = "{\"id\":\"{{ uuid() }}\"}"
target_rps = 250
steady_dur_secs = 15
max_concurrency = 64
out_prefix = "profile-run"

[headers]
Authorization = "Bearer from-file"
"#,
    )
    .expect("write load profile");
    path
}
