use indexmap::IndexMap;
use parking_lot::Mutex;
use rustress_core::snapshot::StatsSnapshot;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use crate::histogram::LatencyHistogram;
use crate::percentiles::PercentileExt;

/// Thread-safe stats accumulator for load test metrics.
///
/// Uses atomic counters for the hot path and mutex-protected maps
/// for status code and error tracking.
pub struct StatsCollector {
    // Atomic counters — lock-free hot path.
    requests: AtomicU64,
    success: AtomicU64,
    fail: AtomicU64,
    bytes: AtomicU64,
    total_queue_wait_micros: AtomicI64,

    // Histograms — mutex-protected.
    service_time: LatencyHistogram,
    total_time: LatencyHistogram,

    // Maps — mutex-protected.
    status_codes: Mutex<IndexMap<u16, u64>>,
    error_counts: Mutex<IndexMap<String, u64>>,
    response_samples: Mutex<IndexMap<u16, String>>,
}

impl StatsCollector {
    /// Create a new stats collector with zeroed counters.
    pub fn new() -> Self {
        Self {
            requests: AtomicU64::new(0),
            success: AtomicU64::new(0),
            fail: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            total_queue_wait_micros: AtomicI64::new(0),
            service_time: LatencyHistogram::new(),
            total_time: LatencyHistogram::new(),
            status_codes: Mutex::new(IndexMap::new()),
            error_counts: Mutex::new(IndexMap::new()),
            response_samples: Mutex::new(IndexMap::new()),
        }
    }

    /// Record a single request outcome.
    ///
    /// This is the hot-path method called after every request completes.
    pub fn add(
        &self,
        success: bool,
        resp_bytes: u64,
        service_time: Duration,
        queue_wait: Duration,
        total_time: Duration,
        status_code: u16,
        error: Option<&str>,
        response_body: Option<&str>,
    ) {
        // Atomic counters — no lock needed.
        self.requests.fetch_add(1, Ordering::Relaxed);
        if success {
            self.success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.fail.fetch_add(1, Ordering::Relaxed);
        }
        self.bytes.fetch_add(resp_bytes, Ordering::Relaxed);
        self.total_queue_wait_micros
            .fetch_add(queue_wait.as_micros() as i64, Ordering::Relaxed);

        // Histograms — single lock each.
        self.service_time
            .record(service_time.as_micros() as u64);
        self.total_time.record(total_time.as_micros() as u64);

        // Maps — single lock for all.
        if let Some(err) = error {
            let mut errors = self.error_counts.lock();
            *errors.entry(err.to_string()).or_insert(0) += 1;

            // Store a sample response body for this error.
            if let Some(body) = response_body {
                let mut samples = self.response_samples.lock();
                samples
                    .entry(0)
                    .or_insert_with(|| body.chars().take(200).collect());
            }
        } else {
            let mut codes = self.status_codes.lock();
            *codes.entry(status_code).or_insert(0) += 1;

            // Also capture response body samples for HTTP >= 400.
            if status_code >= 400 {
                if let Some(body) = response_body {
                    let mut samples = self.response_samples.lock();
                    if !samples.contains_key(&status_code) {
                        samples.insert(status_code, body.chars().take(200).collect());
                    }
                }
            }
        }
    }

    /// Get a snapshot of current stats for the TUI.
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            success: self.success.load(Ordering::Relaxed),
            fail: self.fail.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            inflight: 0, // Set by runner
            p50_service_ms: self.service_time.p50_ms(),
            p90_service_ms: self.service_time.p90_ms(),
            p95_service_ms: self.service_time.p95_ms(),
            p99_service_ms: self.service_time.p99_ms(),
            max_service_ms: self.service_time.max_ms(),
            mean_service_ms: self.service_time.mean_ms(),
            avg_queue_wait_ms: self.avg_queue_wait_ms(),
            status_codes: self.status_codes.lock().clone(),
            error_counts: self.error_counts.lock().clone(),
            response_samples: self.response_samples.lock().clone(),
        }
    }

    /// Average queue wait time in milliseconds.
    pub fn avg_queue_wait_ms(&self) -> f64 {
        let total_micros = self.total_queue_wait_micros.load(Ordering::Relaxed);
        let count = self.requests.load(Ordering::Relaxed);
        if count == 0 {
            return 0.0;
        }
        (total_micros as f64 / count as f64) / 1000.0
    }

    /// Total request count.
    pub fn request_count(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Success count.
    pub fn success_count(&self) -> u64 {
        self.success.load(Ordering::Relaxed)
    }

    /// Failure count.
    pub fn fail_count(&self) -> u64 {
        self.fail.load(Ordering::Relaxed)
    }

    /// Total bytes transferred.
    pub fn total_bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.requests.store(0, Ordering::Relaxed);
        self.success.store(0, Ordering::Relaxed);
        self.fail.store(0, Ordering::Relaxed);
        self.bytes.store(0, Ordering::Relaxed);
        self.total_queue_wait_micros.store(0, Ordering::Relaxed);

        self.service_time.reset();
        self.total_time.reset();

        self.status_codes.lock().clear();
        self.error_counts.lock().clear();
        self.response_samples.lock().clear();
    }
}

impl Default for StatsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use std::sync::Arc;
    use std::thread;

    // --- LatencyHistogram tests ---

    #[test]
    fn test_histogram_new_is_empty() {
        let hist = LatencyHistogram::new();
        assert!(hist.is_empty());
        assert_eq!(hist.len(), 0);
    }

    #[test]
    fn test_histogram_record_single_value() {
        let hist = LatencyHistogram::new();
        hist.record(1000);
        assert!(!hist.is_empty());
        assert_eq!(hist.len(), 1);
    }

    #[test]
    fn test_histogram_percentiles() {
        let hist = LatencyHistogram::new();
        for _ in 0..100 {
            hist.record(1000);
        }
        for _ in 0..100 {
            hist.record(2000);
        }

        // Verify ordering and rough values.
        // HDR histogram with 3 sig figs gives approximate percentiles.
        let p50 = hist.p50_ms();
        let p99 = hist.p99_ms();
        let mean = hist.mean_ms();

        // p50 should be between 1ms and 2ms (we have equal 1ms and 2ms values)
        assert!(p50 >= 1.0 && p50 <= 2.5, "p50 was {p50}");
        // p99 should be close to 2ms
        assert!(p99 >= 1.5 && p99 <= 3.0, "p99 was {p99}");
        // Mean should be around 1.5
        assert!(mean >= 1.0 && mean <= 2.0, "mean was {mean}");
        assert_eq!(hist.max_ms(), 2.0);
        assert_eq!(hist.min_ms(), 1.0);
    }

    #[test]
    fn test_histogram_reset() {
        let hist = LatencyHistogram::new();
        hist.record(5000);
        assert!(!hist.is_empty());

        hist.reset();
        assert!(hist.is_empty());
        assert_eq!(hist.len(), 0);
    }

    #[test]
    fn test_histogram_p99_single_value() {
        let hist = LatencyHistogram::new();
        hist.record(500);
        assert_eq!(hist.p99_ms(), 0.5);
    }

    // --- StatsCollector tests ---

    #[test]
    fn test_collector_initial_state() {
        let collector = StatsCollector::new();
        let snap = collector.snapshot();
        assert_eq!(snap.requests, 0);
        assert_eq!(snap.success, 0);
        assert_eq!(snap.fail, 0);
        assert_eq!(snap.bytes, 0);
    }

    #[test]
    fn test_collector_add_success() {
        let collector = StatsCollector::new();
        collector.add(
            true,
            1024,
            Duration::from_millis(50),
            Duration::from_micros(100),
            Duration::from_millis(51),
            200,
            None,
            None,
        );

        let snap = collector.snapshot();
        assert_eq!(snap.requests, 1);
        assert_eq!(snap.success, 1);
        assert_eq!(snap.fail, 0);
        assert_eq!(snap.bytes, 1024);
        assert!(snap.status_codes.contains_key(&200));
    }

    #[test]
    fn test_collector_add_failure() {
        let collector = StatsCollector::new();
        collector.add(
            false,
            0,
            Duration::from_millis(30),
            Duration::ZERO,
            Duration::from_millis(30),
            500,
            Some("connection refused"),
            Some("error body"),
        );

        let snap = collector.snapshot();
        assert_eq!(snap.fail, 1);
        assert!(snap.error_counts.contains_key("connection refused"));
    }

    #[test]
    fn test_collector_status_codes() {
        let collector = StatsCollector::new();

        collector.add(true, 100, Duration::from_millis(10), Duration::ZERO, Duration::from_millis(10), 200, None, None);
        collector.add(true, 100, Duration::from_millis(10), Duration::ZERO, Duration::from_millis(10), 200, None, None);
        collector.add(true, 100, Duration::from_millis(10), Duration::ZERO, Duration::from_millis(10), 404, None, Some("not found"));
        collector.add(true, 100, Duration::from_millis(10), Duration::ZERO, Duration::from_millis(10), 500, None, Some("server error"));

        let snap = collector.snapshot();
        assert_eq!(snap.status_codes.get(&200), Some(&2));
        assert_eq!(snap.status_codes.get(&404), Some(&1));
        assert_eq!(snap.status_codes.get(&500), Some(&1));
    }

    #[test]
    fn test_collector_response_samples() {
        let collector = StatsCollector::new();
        let body = "x".repeat(300);

        collector.add(
            false,
            0,
            Duration::from_millis(10),
            Duration::ZERO,
            Duration::from_millis(10),
            500,
            Some("internal error"),
            Some(&body),
        );

        let snap = collector.snapshot();
        let sample = snap.response_samples.get(&0).unwrap();
        assert!(sample.len() <= 200);
    }

    #[test]
    fn test_collector_queue_wait() {
        let collector = StatsCollector::new();

        for _ in 0..10 {
            collector.add(
                true,
                0,
                Duration::from_millis(10),
                Duration::from_millis(1),
                Duration::from_millis(11),
                200,
                None,
                None,
            );
        }

        let snap = collector.snapshot();
        assert!((snap.avg_queue_wait_ms - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_collector_reset() {
        let collector = StatsCollector::new();
        collector.add(true, 1024, Duration::from_millis(50), Duration::ZERO, Duration::from_millis(50), 200, None, None);

        collector.reset();

        let snap = collector.snapshot();
        assert_eq!(snap.requests, 0);
        assert_eq!(snap.success, 0);
        assert!(snap.status_codes.is_empty());
    }

    #[test]
    fn test_collector_concurrent_add() {
        let collector = Arc::new(StatsCollector::new());
        let mut handles = vec![];

        for _ in 0..100 {
            let c = Arc::clone(&collector);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    c.add(
                        true,
                        64,
                        Duration::from_micros(500),
                        Duration::ZERO,
                        Duration::from_micros(500),
                        200,
                        None,
                        None,
                    );
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let snap = collector.snapshot();
        assert_eq!(snap.requests, 100_000);
        assert_eq!(snap.success, 100_000);
        assert_eq!(snap.fail, 0);
    }
}
