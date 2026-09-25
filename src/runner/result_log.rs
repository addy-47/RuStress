use parking_lot::Mutex;
use std::collections::VecDeque;

use crate::core::constants::RESULT_RING_CAPACITY;
use crate::core::result::ExperimentResult;

/// Fixed-capacity, overwrite-oldest log of per-request results.
///
/// Retained results exist solely so that CSV/JSON report export has samples to
/// write. Aggregate accuracy is owned by [`crate::metrics::StatsCollector`],
/// which is unbounded-count but O(1) in memory. This buffer deliberately trades
/// report completeness for a hard memory ceiling: resident memory is a function
/// of `capacity`, never of how many requests were executed.
///
/// A run that executes more requests than `capacity` reports
/// [`ResultLog::dropped_from_front`] so callers can state the truncation
/// rather than silently emitting a partial report.
pub struct ResultLog {
    inner: Mutex<Inner>,
    capacity: usize,
}

struct Inner {
    buf: VecDeque<ExperimentResult>,
    /// Number of results evicted because the buffer was full.
    dropped: u64,
}

impl ResultLog {
    /// Create a log retaining at most `capacity` results.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                buf: VecDeque::with_capacity(capacity.min(1_024)),
                dropped: 0,
            }),
            capacity: capacity.max(1),
        }
    }

    /// Append a result, evicting the oldest entry when at capacity.
    pub fn push(&self, result: ExperimentResult) {
        let mut inner = self.inner.lock();
        if inner.buf.len() == self.capacity {
            inner.buf.pop_front();
            inner.dropped += 1;
        }
        inner.buf.push_back(result);
    }

    /// Copy the retained results, oldest first.
    pub fn to_vec(&self) -> Vec<ExperimentResult> {
        self.inner.lock().buf.iter().cloned().collect()
    }

    /// Number of results currently retained.
    pub fn len(&self) -> usize {
        self.inner.lock().buf.len()
    }

    /// Whether no results are retained.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Configured retention capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of results evicted because the buffer was full.
    pub fn dropped_from_front(&self) -> u64 {
        self.inner.lock().dropped
    }

    /// Discard all retained results and reset the drop counter.
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.buf.clear();
        inner.dropped = 0;
    }
}

impl Default for ResultLog {
    fn default() -> Self {
        Self::new(RESULT_RING_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::result::ExperimentResult;
    use chrono::Utc;
    use std::time::Duration;

    fn result(status: u16) -> ExperimentResult {
        ExperimentResult {
            timestamp: Utc::now(),
            latency: Duration::from_millis(1),
            service_time: Duration::from_millis(1),
            queue_wait: Duration::ZERO,
            status,
            success: status < 400,
            bytes: 0,
            user_id: "u".into(),
            query: "custom".into(),
            error: None,
            response_body: None,
        }
    }

    #[test]
    fn empty_log_reports_empty() {
        let log = ResultLog::new(8);
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert_eq!(log.dropped_from_front(), 0);
    }

    #[test]
    fn retains_up_to_capacity_without_dropping() {
        let log = ResultLog::new(4);
        for i in 0..4 {
            log.push(result(200 + i as u16));
        }
        assert_eq!(log.len(), 4);
        assert_eq!(log.dropped_from_front(), 0);
    }

    #[test]
    fn evicts_oldest_and_counts_drops_at_capacity() {
        let log = ResultLog::new(3);
        for i in 0..10 {
            log.push(result(200 + i as u16));
        }
        assert_eq!(log.len(), 3, "buffer must never exceed capacity");
        assert_eq!(log.dropped_from_front(), 7);
    }

    #[test]
    fn retains_most_recent_results_after_eviction() {
        let log = ResultLog::new(3);
        for i in 0..10 {
            log.push(result(200 + i as u16));
        }
        let retained = log.to_vec();
        let statuses: Vec<u16> = retained.iter().map(|r| r.status).collect();
        assert_eq!(statuses, vec![207, 208, 209], "must keep newest, drop oldest");
    }

    #[test]
    fn memory_is_bounded_under_sustained_load() {
        let log = ResultLog::new(16);
        for i in 0..100_000 {
            log.push(result(200 + (i % 100) as u16));
        }
        assert_eq!(log.len(), 16);
        assert_eq!(log.capacity(), 16);
        assert_eq!(log.dropped_from_front(), 99_984);
    }

    #[test]
    fn clear_resets_buffer_and_drop_count() {
        let log = ResultLog::new(2);
        for i in 0..5 {
            log.push(result(200 + i as u16));
        }
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.dropped_from_front(), 0);
    }

    #[test]
    fn capacity_is_at_least_one() {
        let log = ResultLog::new(0);
        log.push(result(200));
        assert_eq!(log.len(), 1, "zero capacity must not panic or drop everything");
    }
}
