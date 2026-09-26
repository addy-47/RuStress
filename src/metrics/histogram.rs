use crate::core::constants::{HISTOGRAM_HIGH_US, HISTOGRAM_LOW_US, HISTOGRAM_SIGFIGS};
use hdrhistogram::Histogram;
use parking_lot::Mutex;

/// Thread-safe HDR histogram wrapper for latency recording.
///
/// Configured for 1 microsecond to 10 minutes range with 3 significant figures.
#[derive(Debug)]
pub struct LatencyHistogram {
    inner: Mutex<Histogram<u64>>,
}

impl LatencyHistogram {
    /// Create a new HDR histogram with the standard range.
    pub fn new() -> Self {
        let inner = Histogram::<u64>::new_with_bounds(
            HISTOGRAM_LOW_US,
            HISTOGRAM_HIGH_US,
            HISTOGRAM_SIGFIGS,
        )
        .expect("valid HDR histogram parameters");

        Self {
            inner: Mutex::new(inner),
        }
    }

    /// Record a latency value in microseconds.
    ///
    /// `record` returns `Err` and stores nothing above
    /// `HISTOGRAM_HIGH_US`, which would drop the very worst latencies from
    /// every percentile while `requests` and `fail` still counted them —
    /// reporting a healthy p99 for the requests that went worst.
    /// `saturating_record` clamps instead, so the sample survives at the
    /// ceiling and p99/max still see it.
    pub fn record(&self, value_us: u64) {
        let mut hist = self.inner.lock();
        hist.saturating_record(value_us);
    }

    /// Get value at a given quantile (0.0–100.0).
    pub fn value_at_quantile(&self, quantile: f64) -> u64 {
        let hist = self.inner.lock();
        hist.value_at_quantile(quantile)
    }

    /// Get the mean value in microseconds.
    pub fn mean(&self) -> f64 {
        let hist = self.inner.lock();
        hist.mean()
    }

    /// Get the max recorded value in microseconds.
    pub fn max(&self) -> u64 {
        let hist = self.inner.lock();
        hist.max()
    }

    /// Get the min recorded value in microseconds.
    pub fn min(&self) -> u64 {
        let hist = self.inner.lock();
        hist.min()
    }

    /// Get the total count of recorded values.
    pub fn len(&self) -> u64 {
        let hist = self.inner.lock();
        hist.len()
    }

    /// Check if no values have been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reset the histogram to empty.
    pub fn reset(&self) {
        self.inner.lock().reset();
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::percentiles::PercentileExt;

    #[test]
    fn a_sample_above_the_ceiling_is_kept_at_the_ceiling() {
        let hist = LatencyHistogram::new();
        hist.record(HISTOGRAM_HIGH_US * 2);

        assert_eq!(
            hist.len(),
            1,
            "the worst possible sample must be stored; `record` returns Err above \
             the ceiling and stores nothing, which drops the requests that hurt \
             most out of every percentile while `requests` still counts them"
        );
        assert!(
            hist.max() >= HISTOGRAM_HIGH_US,
            "the clamped sample must land at the ceiling, got {}",
            hist.max()
        );
        assert!(
            hist.p99_ms() >= HISTOGRAM_HIGH_US as f64 / 1000.0 * 0.99,
            "p99 must reflect the worst sample, got {}",
            hist.p99_ms()
        );
    }

    #[test]
    fn a_sub_microsecond_sample_is_still_counted() {
        let hist = LatencyHistogram::new();
        hist.record(0);

        assert_eq!(
            hist.len(),
            1,
            "a zero-microsecond service time is a real measurement and must be \
             counted, or `requests` and the histogram disagree about how much \
             traffic the run produced"
        );
        assert_eq!(
            hist.max(),
            0,
            "characterisation, not endorsement: hdrhistogram stores 0 at index 0 \
             and leaves its max unchanged, so a sub-microsecond request is \
             counted but reports a zero service time. The value is bounded below \
             by 1us in practice, so this is display precision, not data loss."
        );
    }

    #[test]
    fn a_sample_exactly_at_the_ceiling_is_kept_without_clamping() {
        let hist = LatencyHistogram::new();
        hist.record(HISTOGRAM_HIGH_US);

        assert_eq!(hist.len(), 1);
        assert!(hist.max() >= HISTOGRAM_HIGH_US);
        assert!(
            hist.p99_ms() >= HISTOGRAM_HIGH_US as f64 / 1000.0 * 0.99,
            "p99 at the exact ceiling must reflect it, got {}",
            hist.p99_ms()
        );
    }

    #[test]
    fn an_empty_histogram_reports_nothing_recorded() {
        let hist = LatencyHistogram::new();
        assert!(hist.is_empty());
        assert_eq!(hist.len(), 0);
        assert_eq!(hist.min(), 0);
    }

    #[test]
    fn reset_empties_the_histogram() {
        let hist = LatencyHistogram::new();
        hist.record(1_000);
        hist.reset();
        assert!(hist.is_empty());
        assert_eq!(hist.p99_ms(), 0.0);
    }
}
