//! ============================================================================
//! common/mod.rs — shared real-HTTP-server harness for the integration suite
//! ============================================================================
//! Category     : Integration Test (shared support module)
//! Component    : test harness for `engine -> executor -> HTTP -> metrics`
//! Prerequisites: none (every server binds an OS-assigned ephemeral port)
//! Execution    : cargo test --test <name>
//! Metrics      : per-test; this module records none of its own
//! ============================================================================
//!
//! Zero-mock rule: every target here is a real HTTP server on a real socket.
//! `serve` binds `127.0.0.1:0`, so no test races another for a port and no test
//! can leave a listener behind for the next one.

#![allow(dead_code)]
// Each integration target links this module and uses a subset of it, so every
// helper is dead code in the targets that do not call it.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use rustress::core::config::Config;
use rustress::core::constants::STATS_CHANNEL_CAPACITY;
use rustress::core::result::ExperimentResult;
use rustress::core::snapshot::StatsSnapshot;
use rustress::runner::LoadEngine;
use rustress::runner::stats::RunStats;

/// A real HTTP server listening on an OS-assigned ephemeral port.
pub struct TestServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
}

impl TestServer {
    /// Socket address the OS actually assigned.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Absolute URL for `path` on this server.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // `axum::serve` has no graceful shutdown handle here, and a leaked
        // listener would make the next test's ephemeral-port bind ambiguous.
        self.handle.abort();
    }
}

/// Serve `router` on an ephemeral port and return its real address.
pub async fn serve(router: Router) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("read bound address");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    TestServer { addr, handle }
}

/// Stand up the crate's own reference target on an ephemeral port.
pub async fn dummy_server() -> TestServer {
    serve(rustress::dummy::server::DummyServer::router()).await
}

/// A `LoadEngine` bound to a private snapshot channel, plus its cancel token.
pub struct Harness {
    engine: LoadEngine,
    pub updates: mpsc::Receiver<StatsSnapshot>,
    cancel: CancellationToken,
}

impl Harness {
    /// Build an engine through the real production constructor.
    ///
    /// Errors propagate rather than panicking so the config-validation tests
    /// can assert on the rejection instead of on a panic message.
    pub fn new(cfg: Config) -> anyhow::Result<Self> {
        let (tx, updates) = mpsc::channel(STATS_CHANNEL_CAPACITY);
        let engine = LoadEngine::new(cfg, tx)?;
        Ok(Self {
            engine,
            updates,
            cancel: CancellationToken::new(),
        })
    }

    /// A clone of the token that stops the run.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Run to completion or cancellation and return only once drained.
    pub async fn run(&self) {
        self.engine.run(self.cancel.clone()).await;
    }

    /// Run, firing `cancel` after `after`, and return only once drained.
    pub async fn run_cancelling_after(&self, after: Duration) {
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            cancel.cancel();
        });
        self.engine.run(self.cancel.clone()).await;
    }

    /// Live shared metrics for this run.
    pub fn stats(&self) -> &Arc<RunStats> {
        self.engine.stats()
    }

    /// Authoritative counters, read from the engine rather than the channel.
    ///
    /// The channel carries frames produced during the run; the final frame is
    /// published *during* drain, after a reader would normally have stopped.
    pub fn snapshot(&self) -> StatsSnapshot {
        self.engine.stats().snapshot()
    }

    /// Per-request results retained by the bounded ring, oldest first.
    pub fn results(&self) -> Vec<ExperimentResult> {
        self.engine.stats().get_results()
    }
}

/// A config pointed at `url`, sized so a full run stays inside a few seconds.
pub fn cfg_for(url: &str) -> Config {
    Config {
        url: url.to_string(),
        target_rps: 20,
        steady_dur_secs: 1,
        timeout_secs: 5,
        max_concurrency: 32,
        ..Default::default()
    }
}

/// Current resident set size in bytes, read from the kernel.
///
/// `None` off Linux: there is no portable equivalent, and a guessed figure
/// would be worse than an honest absence.
pub fn rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// Fail a test that would otherwise pass without exercising the property.
pub fn assert_grew_by_at_least(label: &str, before: u64, after: u64, floor: u64) {
    assert!(
        after.saturating_sub(before) >= floor,
        "{label}: resident memory grew by only {} bytes, but this probe must be \
         able to observe at least {floor} bytes of real growth. A probe that \
         cannot see a known allocation cannot certify an allocation bound.",
        after.saturating_sub(before)
    );
}
