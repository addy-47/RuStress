use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use tokio::sync::mpsc;

use crate::core::result::ExperimentResult;
use crate::core::snapshot::StatsSnapshot;
use crate::metrics::StatsCollector;
use crate::runner::result_log::ResultLog;

/// Shared, thread-safe metric state for a single load run.
///
/// Aggregate counters live in [`StatsCollector`]. Per-request results live in a
/// fixed-capacity [`ResultLog`], so total memory is bounded by the configured
/// retention capacity regardless of how many requests execute.
pub struct RunStats {
    pub collector: Arc<StatsCollector>,
    pub results: ResultLog,
    pub inflight: Arc<AtomicI64>,
    /// Requests the scheduler could not dispatch because the concurrency
    /// ceiling was saturated. Non-zero means the generator, not the target,
    /// was the bottleneck.
    pub dropped_scheduled: Arc<AtomicU64>,
    updates: mpsc::UnboundedSender<StatsSnapshot>,
}

impl RunStats {
    pub fn new(updates: mpsc::UnboundedSender<StatsSnapshot>) -> Self {
        Self {
            collector: Arc::new(StatsCollector::new()),
            results: ResultLog::default(),
            inflight: Arc::new(AtomicI64::new(0)),
            dropped_scheduled: Arc::new(AtomicU64::new(0)),
            updates,
        }
    }

    /// Count a request the scheduler admitted to the runtime.
    ///
    /// This is incremented at *dispatch* time, not when the request task
    /// begins executing. Counting at task start would make queued tasks
    /// invisible to the drain barrier, and the run would abandon them.
    pub fn inc_inflight(&self) {
        self.inflight.fetch_add(1, Ordering::SeqCst);
    }

    pub fn dec_inflight(&self) {
        self.inflight.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn inflight_count(&self) -> i64 {
        self.inflight.load(Ordering::SeqCst)
    }

    /// Count a request the scheduler could not dispatch due to saturation.
    pub fn record_scheduled_drop(&self) {
        self.record_scheduled_drops(1);
    }

    /// Count `count` shed requests in a single atomic operation.
    ///
    /// A batched add matters when resynchronising the schedule: the number of
    /// skipped slots is `elapsed / period`, which reaches millions at high
    /// RPS, and one `fetch_add` per slot would spin on the scheduler task
    /// itself — manufacturing the very slip it is accounting for.
    pub fn record_scheduled_drops(&self, count: u64) {
        if count > 0 {
            self.dropped_scheduled.fetch_add(count, Ordering::Relaxed);
        }
    }

    /// Total scheduler-side drops for this run.
    pub fn dropped_scheduled_count(&self) -> u64 {
        self.dropped_scheduled.load(Ordering::Relaxed)
    }

    /// Fold a completed request into both aggregate counters and the result log.
    pub fn record(&self, result: ExperimentResult) {
        self.collector.add(&result);
        self.results.push(result);
    }

    /// Build a snapshot for the TUI, including live in-flight, drop, and
    /// eviction counts.
    pub fn snapshot(&self) -> StatsSnapshot {
        let mut snap = self.collector.snapshot();
        snap.inflight = self.inflight_count();
        snap.dropped_scheduled = self.dropped_scheduled_count();
        snap.dropped_results = self.results.dropped_from_front();
        snap
    }

    /// Retained per-request results, oldest first.
    pub fn get_results(&self) -> Vec<ExperimentResult> {
        self.results.to_vec()
    }

    /// Emit a snapshot to the UI channel.
    pub fn publish_snapshot(&self) {
        let _ = self.updates.send(self.snapshot());
    }
}

/// Decrements the in-flight count when dropped.
///
/// Inflight accounting must release on every exit path — completion, early
/// return, or panic — or the drain barrier waits forever. A `Drop` guard is the
/// only construct that guarantees all three.
pub struct InflightGuard {
    stats: Arc<RunStats>,
}

impl InflightGuard {
    /// Admit one unit of work and return the guard that releases it.
    pub fn admit(stats: &Arc<RunStats>) -> Self {
        stats.inc_inflight();
        Self {
            stats: Arc::clone(stats),
        }
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.stats.dec_inflight();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::constants::RESULT_RING_CAPACITY;
    use std::panic;

    fn stats() -> Arc<RunStats> {
        let (tx, _rx) = mpsc::unbounded_channel();
        Arc::new(RunStats::new(tx))
    }

    #[test]
    fn inflight_returns_to_zero_when_guards_drop() {
        let s = stats();
        {
            let _a = InflightGuard::admit(&s);
            let _b = InflightGuard::admit(&s);
            assert_eq!(s.inflight_count(), 2);
        }
        assert_eq!(s.inflight_count(), 0);
    }

    #[test]
    fn guard_releases_on_panic() {
        let s = stats();
        let result = std::panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = InflightGuard::admit(&s);
            panic!("request task blew up");
        }));
        assert!(result.is_err());
        assert_eq!(
            s.inflight_count(),
            0,
            "a panicking request must not wedge the drain barrier forever"
        );
    }

    #[test]
    fn snapshot_reports_drops_and_evictions() {
        let s = stats();
        s.record_scheduled_drop();
        s.record_scheduled_drop();

        for i in 0..(RESULT_RING_CAPACITY + 5) {
            s.record(sample(i as u16));
        }

        let snap = s.snapshot();
        assert_eq!(snap.dropped_scheduled, 2);
        assert_eq!(snap.dropped_results, 5);
        assert_eq!(snap.requests, (RESULT_RING_CAPACITY + 5) as u64);
    }

    fn sample(status: u16) -> ExperimentResult {
        ExperimentResult {
            timestamp: chrono::Utc::now(),
            latency: std::time::Duration::from_millis(1),
            service_time: std::time::Duration::from_millis(1),
            queue_wait: std::time::Duration::ZERO,
            status,
            success: status < 400,
            bytes: 0,
            user_id: "u".into(),
            query: "custom".into(),
            error: None,
            response_body: None,
        }
    }
}
