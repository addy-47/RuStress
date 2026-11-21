//! ============================================================================
//! cli_output_test.rs — headless output is script-safe and cookies reach the wire
//! ============================================================================
//! Category     : Integration Test
//! Component    : `cli::args::Cli`, `cli::commands::headless::run_headless`
//! Prerequisites: none (real axum target on an ephemeral port)
//! Execution    : cargo test --test cli_output_test
//! Metrics      : captured stdout/stderr, Cookie header observed by the server
//! ============================================================================
//!
//! These tests drive the shipped binary, not a helper that mimics it. stdout
//! is a pipe, so indicatif's own non-TTY hiding is already active; the control
//! case proves the ANSI banner is suppressed on a pipe even without --quiet,
//! and the quiet case proves the flag removes the plain-text preamble while
//! leaving the authoritative summary and the shed warning intact.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use tokio::process::Command;

use common::serve;

#[derive(Clone, Default)]
struct SeenCookies(Arc<Mutex<Vec<String>>>);

async fn record_cookie(State(seen): State<SeenCookies>, headers: HeaderMap) -> &'static str {
    if let Some(value) = headers.get(axum::http::header::COOKIE) {
        seen.0
            .lock()
            .expect("cookie lock")
            .push(value.to_str().unwrap_or_default().to_string());
    }
    "ok"
}

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rustress"))
}

/// Bound every subprocess: a hung target must fail the test, not the suite.
async fn output_of(cmd: &mut Command) -> std::process::Output {
    tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .expect("the CLI must finish within 30s")
        .expect("spawn the CLI")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiet_emits_the_summary_without_preamble_or_ansi() {
    let server = serve(Router::new().route("/fast", get(|| async { "ok" }))).await;

    let out = output_of(binary().args([
        "--url",
        &server.url("/fast"),
        "--rate",
        "20",
        "--duration",
        "1",
        "--quiet",
    ]))
    .await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("URL:"), "preamble leaked: {stdout}");
    assert!(!stdout.contains('\u{1b}'), "ANSI escape leaked: {stdout:?}");
    assert!(
        stdout.contains("RESULTS"),
        "the summary must survive: {stdout}"
    );
    assert!(
        stdout.contains("Requests:"),
        "counters must survive: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pipe_suppresses_the_ansi_banner_even_without_quiet() {
    let server = serve(Router::new().route("/fast", get(|| async { "ok" }))).await;

    let out = output_of(binary().args([
        "--url",
        &server.url("/fast"),
        "--rate",
        "20",
        "--duration",
        "1",
    ]))
    .await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("URL:"),
        "the control case keeps the preamble"
    );
    assert!(
        !stdout.contains('\u{1b}'),
        "ANSI escapes must not enter a piped log: {stdout:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_shed_warning_survives_quiet_mode() {
    let server = serve(rustress::dummy::server::DummyServer::router()).await;

    let out = output_of(binary().args([
        "--url",
        &server.url("/slow"),
        "--rate",
        "10000",
        "--max-concurrency",
        "1",
        "--duration",
        "1",
        "--quiet",
    ]))
    .await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("MEASUREMENT INVALID"),
        "shedding must invalidate the run even under --quiet: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookie_values_reach_a_real_target() {
    let seen = SeenCookies::default();
    let server =
        serve(Router::new().route("/fast", get(record_cookie).with_state(seen.clone()))).await;

    let out = output_of(binary().args([
        "--url",
        &server.url("/fast"),
        "--rate",
        "20",
        "--duration",
        "1",
        "--cookie",
        "sid=abc123; theme=dark",
        "--cookie",
        "csrf=abc==",
        "--quiet",
    ]))
    .await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let captured = seen.0.lock().expect("cookie lock").clone();
    assert!(!captured.is_empty(), "the target saw no Cookie header");
    assert!(
        captured
            .iter()
            .all(|v| v == "sid=abc123; theme=dark; csrf=abc=="),
        "every request must carry the merged cookie: {captured:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_cookie_is_a_hard_error() {
    let out = output_of(binary().args([
        "--url",
        "http://127.0.0.1:9/x",
        "--cookie",
        "session",
        "--quiet",
    ]))
    .await;

    assert!(
        !out.status.success(),
        "a broken cookie must not start a run"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("invalid --cookie"),
        "the diagnostic must name the flag: {stderr}"
    );
}
