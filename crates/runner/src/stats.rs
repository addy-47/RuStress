use parking_lot::Mutex;
use rustress_core::result::ExperimentResult;
use rustress_core::snapshot::StatsSnapshot;
use rustress_metrics::StatsCollector;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Shared stats container for the runner.
pub struct RunStats {
    pub collector: Arc<StatsCollector>,
    pub results: Mutex<Vec<ExperimentResult>>,
    pub inflight: Arc<AtomicI64>,
    pub updates: mpsc::UnboundedSender<StatsSnapshot>,
}

impl RunStats {
    pub fn new(updates: mpsc::UnboundedSender<StatsSnapshot>) -> Self {
        Self {
            collector: Arc::new(StatsCollector::new()),
            results: Mutex::new(Vec::new()),
            inflight: Arc::new(AtomicI64::new(0)),
            updates,
        }
    }

    pub fn inc_inflight(&self) {
        self.inflight.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_inflight(&self) {
        self.inflight.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn inflight_count(&self) -> i64 {
        self.inflight.load(Ordering::Relaxed)
    }

    /// Record a result (adds to collector and results list).
    pub fn record(&self, result: &ExperimentResult) {
        self.collector.add(
            result.success,
            result.bytes.max(0) as u64,
            result.service_time,
            result.queue_wait,
            result.latency,
            result.status,
            result.error.as_deref(),
            result.response_body.as_deref(),
        );

        self.results.lock().push(result.clone());
    }

    /// Get a snapshot of current stats.
    pub fn snapshot(&self) -> StatsSnapshot {
        let mut snap = self.collector.snapshot();
        snap.inflight = self.inflight_count();
        snap
    }

    /// Get all collected results.
    pub fn get_results(&self) -> Vec<ExperimentResult> {
        self.results.lock().clone()
    }
}
