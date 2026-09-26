//! ============================================================================
//! controller.rs — load-run lifecycle, independent of any terminal
//! ============================================================================
//! Component    : runner::LoadEngine lifecycle for the interactive dashboard
//! Prerequisites: none
//!
//! # Why this exists
//!
//! The interactive TUI built a `LoadEngine` and never started it, so `Ctrl+R`
//! showed "RUNNING" over a dashboard that no traffic could reach. The fix is
//! not to sprinkle `tokio::spawn` through the event loop: a TUI cannot be
//! integration-tested in CI because there is no TTY, so orchestration placed
//! there would be untestable orchestration.
//!
//! `RunController` owns the engine's whole lifecycle — build, spawn, cancel,
//! drain, join — and knows nothing about terminals, frames or key events. That
//! makes "does starting a run actually generate traffic?" an ordinary
//! assertion in an ordinary test, against the real `DummyServer`.
//!
//! The TUI drives it; it does not implement it.
//! ============================================================================

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::core::config::Config;
use crate::core::constants::STATS_CHANNEL_CAPACITY;
use crate::core::snapshot::StatsSnapshot;
use crate::runner::LoadEngine;
use crate::runner::stats::RunStats;

/// Owns at most one in-flight load run.
#[derive(Default)]
pub struct RunController {
    handle: Option<JoinHandle<()>>,
    cancel: Option<CancellationToken>,
    stats_rx: Option<mpsc::Receiver<StatsSnapshot>>,
    stats: Option<Arc<RunStats>>,
}

impl RunController {
    /// A controller with no run in flight.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build an engine from `cfg` and start generating load.
    ///
    /// Returns the configuration error rather than aborting, so a bad value
    /// entered in the dashboard surfaces in the status line instead of killing
    /// the process.
    pub fn start(&mut self, cfg: Config) -> Result<(), Vec<String>> {
        if self.is_running() {
            return Err(vec!["a run is already in progress".to_string()]);
        }

        let (tx, rx) = mpsc::channel(STATS_CHANNEL_CAPACITY);
        let engine = LoadEngine::new(cfg, tx).map_err(|e| vec![e.to_string()])?;
        let stats = Arc::clone(engine.stats());
        let cancel = CancellationToken::new();

        let handle = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                engine.run(cancel).await;
            }
        });

        // The engine itself moves into the task; only the shared counters are
        // retained, because a final summary must read them after the task ends.
        self.stats = Some(stats);
        self.handle = Some(handle);
        self.cancel = Some(cancel);
        self.stats_rx = Some(rx);
        Ok(())
    }

    /// Take the newest pending snapshot, if any.
    ///
    /// Drains rather than peeks: the producer ticks faster than the display
    /// refreshes, so a single-frame read would report stale counters — the same
    /// bug that made the headless summary under-report by half.
    pub fn poll(&mut self) -> Option<StatsSnapshot> {
        let rx = self.stats_rx.as_mut()?;
        let mut newest = None;
        while let Ok(snap) = rx.try_recv() {
            newest = Some(snap);
        }
        newest
    }

    /// Signal the run to stop. The drain barrier still applies: in-flight
    /// requests complete before the run reports.
    pub fn stop(&mut self) {
        if let Some(cancel) = self.cancel.as_ref() {
            cancel.cancel();
        }
    }

    /// Cancel and wait for the run to finish draining.
    ///
    /// Must be awaited before the terminal is torn down, so that a `Drop` on
    /// the engine cannot race the `TerminalGuard`.
    pub async fn shutdown(&mut self) {
        self.stop();
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
        self.cancel = None;
        self.stats_rx = None;
    }

    /// Whether a run task is still in flight.
    pub fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// The live counters, for a final summary read after shutdown.
    pub fn stats(&self) -> Option<&Arc<RunStats>> {
        self.stats.as_ref()
    }
}
