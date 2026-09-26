use crate::core::constants::{ERROR_KEY_OVERFLOW_LABEL, MAX_TRACKED_ERROR_KEYS};
use indexmap::IndexMap;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::result::ExperimentResult;
use crate::core::snapshot::StatsSnapshot;
use crate::metrics::histogram::LatencyHistogram;
use crate::metrics::percentiles::PercentileExt;

/// Thread-safe stats accumulator for load test metrics.
///
/// Uses atomic counters for the hot path and mutex-protected maps
/// for status code and error tracking. Every map has a bounded key space so
/// that memory is a function of configuration, not of request count.
pub struct StatsCollector {
    // Atomic counters — lock-free hot path.
    requests: AtomicU64,
    success: AtomicU64,
    fail: AtomicU64,
    bytes: AtomicU64,
    total_queue_wait_micros: AtomicU64,

    // Histograms — mutex-protected.
    service_time: LatencyHistogram,

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
            total_queue_wait_micros: AtomicU64::new(0),
            service_time: LatencyHistogram::new(),
            status_codes: Mutex::new(IndexMap::new()),
            error_counts: Mutex::new(IndexMap::new()),
            response_samples: Mutex::new(IndexMap::new()),
        }
    }

    /// Record a single completed request.
    ///
    /// This is the hot-path method called after every request completes. It
    /// takes the result struct rather than nine positional arguments so that
    /// adding a measurement cannot silently reorder call sites.
    pub fn add(&self, result: &ExperimentResult) {
        let status_code = result.status;
        let error = result.error.as_deref();
        let response_body = result.response_body.as_deref();

        // Atomic counters — no lock needed.
        self.requests.fetch_add(1, Ordering::Relaxed);
        if result.success {
            self.success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.fail.fetch_add(1, Ordering::Relaxed);
        }
        self.bytes
            .fetch_add(result.bytes.max(0) as u64, Ordering::Relaxed);
        self.total_queue_wait_micros
            .fetch_add(result.queue_wait.as_micros() as u64, Ordering::Relaxed);

        // Histograms — single lock each.
        self.service_time
            .record(result.service_time.as_micros() as u64);

        // Maps.
        //
        // Status and error are recorded independently, not either/or. A
        // transport failure has no status (0) but does have an error; a
        // response that failed mid-body has BOTH a real observed status and an
        // error. Branching on one suppressed the other.
        if status_code != 0 {
            let mut codes = self.status_codes.lock();
            *codes.entry(status_code).or_insert(0) += 1;

            if status_code >= 400 {
                if let Some(body) = response_body {
                    let mut samples = self.response_samples.lock();
                    samples
                        .entry(status_code)
                        .or_insert_with(|| sample_of(body));
                }
            }
        }

        if let Some(err) = error {
            let mut errors = self.error_counts.lock();
            let key = bounded_error_key(&errors, err);
            *errors.entry(key).or_insert(0) += 1;

            if let Some(body) = response_body {
                let mut samples = self.response_samples.lock();
                samples.entry(0).or_insert_with(|| sample_of(body));
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
            dropped_scheduled: 0,
            dropped_results: 0,
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

/// Truncate a captured body to a diagnostic-sized sample.
fn sample_of(body: &str) -> String {
    body.chars().take(MAX_SAMPLE_CHARS).collect()
}

/// Maximum characters retained from a response body for diagnostics.
const MAX_SAMPLE_CHARS: usize = 200;

/// Map an error message onto a bounded key space.
///
/// Error text originates from remote servers and network stacks, so its
/// cardinality is target-controlled. A target returning a unique error string
/// per request would otherwise grow `error_counts` without limit. Once the
/// tracked key budget is spent, unseen errors fold into a single overflow
/// bucket; already-tracked errors keep incrementing their own key.
fn bounded_error_key(errors: &IndexMap<String, u64>, err: &str) -> String {
    if errors.contains_key(err) || errors.len() < MAX_TRACKED_ERROR_KEYS {
        return err.to_string();
    }
    ERROR_KEY_OVERFLOW_LABEL.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::constants::{MAX_CAPTURED_BODY_BYTES, RESULT_RING_CAPACITY};
    use chrono::Utc;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// Build a result for collector assertions.
    fn result(status: u16, success: bool) -> ExperimentResult {
        ExperimentResult {
            timestamp: Utc::now(),
            latency: Duration::from_millis(11),
            service_time: Duration::from_millis(10),
            queue_wait: Duration::from_millis(1),
            status,
            success,
            bytes: 1024,
            user_id: "u".into(),
            query: "custom".into(),
            error: None,
            response_body: None,
        }
    }

    /// Build a transport-failure result carrying `message`.
    fn error_result(message: &str) -> ExperimentResult {
        let mut r = result(0, false);
        r.error = Some(message.to_string());
        r
    }

    #[test]
    fn new_collector_reports_zeroed_counters() {
        let snap = StatsCollector::new().snapshot();
        assert_eq!(snap.requests, 0);
        assert_eq!(snap.success, 0);
        assert_eq!(snap.fail, 0);
        assert_eq!(snap.bytes, 0);
    }

    #[test]
    fn success_increments_success_counter_and_status_map() {
        let collector = StatsCollector::new();
        collector.add(&result(200, true));

        let snap = collector.snapshot();
        assert_eq!(snap.requests, 1);
        assert_eq!(snap.success, 1);
        assert_eq!(snap.fail, 0);
        assert_eq!(snap.bytes, 1024);
        assert_eq!(snap.status_codes.get(&200), Some(&1));
    }

    #[test]
    fn failure_increments_fail_counter_and_error_map() {
        let collector = StatsCollector::new();
        let mut r = result(500, false);
        r.error = Some("connection refused".into());

        collector.add(&r);

        let snap = collector.snapshot();
        assert_eq!(snap.fail, 1);
        assert_eq!(snap.success, 0);
        assert_eq!(snap.error_counts.get("connection refused"), Some(&1));
    }

    #[test]
    fn transport_errors_do_not_pollute_status_code_map() {
        let collector = StatsCollector::new();
        // A transport failure never reached the wire, so it has no status.
        let mut r = result(0, false);
        r.error = Some("connection refused".into());

        collector.add(&r);

        assert!(
            collector.snapshot().status_codes.is_empty(),
            "a transport error has no status code and must not be recorded as one"
        );
    }

    #[test]
    fn a_mid_body_error_keeps_its_observed_status() {
        let collector = StatsCollector::new();
        // 200 headers, then the body stream failed: the status is real and
        // observed, so it belongs in the distribution.
        let mut r = result(200, false);
        r.error = Some("body stream closed".into());

        collector.add(&r);

        let snap = collector.snapshot();
        assert_eq!(snap.status_codes.get(&200), Some(&1));
        assert_eq!(snap.error_counts.get("body stream closed"), Some(&1));
        assert_eq!(snap.fail, 1);
    }

    #[test]
    fn status_codes_accumulate_per_code() {
        let collector = StatsCollector::new();
        collector.add(&result(200, true));
        collector.add(&result(200, true));
        collector.add(&result(404, true));

        let snap = collector.snapshot();
        assert_eq!(snap.status_codes.get(&200), Some(&2));
        assert_eq!(snap.status_codes.get(&404), Some(&1));
    }

    #[test]
    fn error_body_sample_is_length_capped() {
        let collector = StatsCollector::new();
        let mut r = result(500, false);
        r.error = Some("internal".into());
        r.response_body = Some("x".repeat(5_000));

        collector.add(&r);

        let snap = collector.snapshot();
        let sample = snap.response_samples.get(&0).unwrap();
        assert!(sample.chars().count() <= 200);
    }

    #[test]
    fn avg_queue_wait_divides_by_request_count() {
        let collector = StatsCollector::new();
        for _ in 0..10 {
            let mut r = result(200, true);
            r.queue_wait = Duration::from_millis(1);
            collector.add(&r);
        }

        assert!((collector.avg_queue_wait_ms() - 1.0).abs() < 0.01);
    }

    #[test]
    fn avg_queue_wait_is_zero_with_no_requests() {
        assert_eq!(StatsCollector::new().avg_queue_wait_ms(), 0.0);
    }

    #[test]
    fn reset_clears_counters_and_maps() {
        let collector = StatsCollector::new();
        collector.add(&result(200, true));
        collector.reset();

        let snap = collector.snapshot();
        assert_eq!(snap.requests, 0);
        assert_eq!(snap.success, 0);
        assert!(snap.status_codes.is_empty());
        assert!(snap.error_counts.is_empty());
    }

    #[test]
    fn error_key_space_is_bounded_under_unique_errors() {
        let collector = StatsCollector::new();
        let total = MAX_TRACKED_ERROR_KEYS * 50;
        for i in 0..total {
            let mut r = result(0, false);
            r.error = Some(format!("unique-error-{i}"));
            collector.add(&r);
        }

        let errors = collector.snapshot().error_counts;
        assert_eq!(
            errors.len(),
            MAX_TRACKED_ERROR_KEYS + 1,
            "error map must cap at the budget plus the overflow bucket"
        );
        let overflow = (total - MAX_TRACKED_ERROR_KEYS) as u64;
        assert_eq!(errors.get(ERROR_KEY_OVERFLOW_LABEL), Some(&overflow));
    }

    #[test]
    fn tracked_error_keeps_incrementing_after_budget_spent() {
        let collector = StatsCollector::new();
        for i in 0..(MAX_TRACKED_ERROR_KEYS + 10) {
            let mut r = result(0, false);
            r.error = Some(format!("unique-{i}"));
            collector.add(&r);
        }

        let mut repeat = result(0, false);
        repeat.error = Some("unique-0".into());
        collector.add(&repeat);
        collector.add(&repeat);

        assert_eq!(collector.snapshot().error_counts.get("unique-0"), Some(&3));
    }

    #[test]
    fn concurrent_recording_is_lossless() {
        let collector = Arc::new(StatsCollector::new());
        let threads = 16;
        let per_thread = 2_000;

        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let c = Arc::clone(&collector);
                thread::spawn(move || {
                    for _ in 0..per_thread {
                        c.add(&result(200, true));
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let snap = collector.snapshot();
        assert_eq!(snap.requests, (threads * per_thread) as u64);
        let expected = (threads * per_thread) as u64;
        assert_eq!(snap.success, (threads * per_thread) as u64);
        assert_eq!(snap.fail, 0);
        assert_eq!(snap.status_codes.get(&200), Some(&expected));
    }

    #[test]
    fn histogram_records_into_service_time() {
        let collector = StatsCollector::new();
        for _ in 0..100 {
            let mut r = result(200, true);
            r.service_time = Duration::from_millis(10);
            collector.add(&r);
        }

        let snap = collector.snapshot();
        assert!(snap.p50_service_ms >= 9.0 && snap.p50_service_ms <= 11.0);
        assert!(snap.max_service_ms >= 9.0);
    }

    #[test]
    fn retention_capacity_constant_is_sane() {
        assert_ne!(
            RESULT_RING_CAPACITY, 0,
            "a zero-capacity ring would discard every result"
        );
    }

    #[test]
    fn exactly_the_key_budget_opens_no_overflow_bucket() {
        let collector = StatsCollector::new();
        for i in 0..MAX_TRACKED_ERROR_KEYS {
            collector.add(&error_result(&format!("err-{i}")));
        }

        let errors = collector.snapshot().error_counts;
        assert_eq!(
            errors.len(),
            MAX_TRACKED_ERROR_KEYS,
            "the budget is a ceiling, not a target: filling it exactly must not \
             spill into the overflow bucket"
        );
        assert!(
            !errors.contains_key(ERROR_KEY_OVERFLOW_LABEL),
            "an overflow bucket that exists while unused is a key an operator \
             could mistake for a real error class: {errors:?}"
        );
    }

    #[test]
    fn one_error_past_the_key_budget_lands_in_the_overflow_bucket() {
        let collector = StatsCollector::new();
        for i in 0..=MAX_TRACKED_ERROR_KEYS {
            collector.add(&error_result(&format!("err-{i}")));
        }

        let errors = collector.snapshot().error_counts;
        assert_eq!(errors.len(), MAX_TRACKED_ERROR_KEYS + 1);
        assert_eq!(
            errors.get(ERROR_KEY_OVERFLOW_LABEL),
            Some(&1),
            "exactly one error is past the budget, so the bucket must hold exactly one"
        );
        assert_eq!(
            errors.get("err-0"),
            Some(&1),
            "the in-budget key is unaffected"
        );
    }

    #[test]
    fn the_overflow_bucket_accumulates_rather_than_replacing() {
        let collector = StatsCollector::new();
        for i in 0..MAX_TRACKED_ERROR_KEYS {
            collector.add(&error_result(&format!("err-{i}")));
        }
        assert!(
            !collector
                .snapshot()
                .error_counts
                .contains_key(ERROR_KEY_OVERFLOW_LABEL),
            "the bucket must start empty once the budget is exactly full"
        );

        for i in 0..100 {
            collector.add(&error_result(&format!("later-{i}")));
        }

        assert_eq!(
            collector
                .snapshot()
                .error_counts
                .get(ERROR_KEY_OVERFLOW_LABEL),
            Some(&100),
            "each folded error must add to the same bucket, not reset it"
        );
    }

    #[test]
    fn a_captured_body_does_not_change_the_error_key() {
        let collector = StatsCollector::new();
        let mut with_body = error_result("connection refused");
        with_body.response_body = Some("x".repeat(MAX_CAPTURED_BODY_BYTES));

        collector.add(&error_result("connection refused"));
        collector.add(&with_body);

        let errors = collector.snapshot().error_counts;
        assert_eq!(errors.len(), 1, "the same error must not become two keys");
        assert_eq!(errors.get("connection refused"), Some(&2));
    }

    #[test]
    fn bytes_counter_clamps_a_negative_result() {
        let collector = StatsCollector::new();
        let mut r = result(200, true);
        r.bytes = -5_000;
        collector.add(&r);

        assert_eq!(
            collector.total_bytes(),
            0,
            "a negative byte count must not be added, or the throughput figure \
             under-reports in a way nobody can explain"
        );
    }
}
