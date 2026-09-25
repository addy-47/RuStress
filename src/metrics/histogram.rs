use hdrhistogram::Histogram;
use parking_lot::Mutex;
use crate::core::constants::{HISTOGRAM_HIGH_US, HISTOGRAM_LOW_US, HISTOGRAM_SIGFIGS};

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
    pub fn record(&self, value_us: u64) {
        let mut hist = self.inner.lock();
        let _ = hist.record(value_us);
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
