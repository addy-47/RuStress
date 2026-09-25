use crate::histogram::LatencyHistogram;

/// Percentile extraction helper for latency histograms.
///
/// All methods return values in **milliseconds** (histogram stores microseconds).
pub trait PercentileExt {
    fn p50_ms(&self) -> f64;
    fn p90_ms(&self) -> f64;
    fn p95_ms(&self) -> f64;
    fn p99_ms(&self) -> f64;
    fn mean_ms(&self) -> f64;
    fn max_ms(&self) -> f64;
    fn min_ms(&self) -> f64;
}

impl PercentileExt for LatencyHistogram {
    fn p50_ms(&self) -> f64 {
        self.value_at_quantile(50.0) as f64 / 1000.0
    }

    fn p90_ms(&self) -> f64 {
        self.value_at_quantile(90.0) as f64 / 1000.0
    }

    fn p95_ms(&self) -> f64 {
        self.value_at_quantile(95.0) as f64 / 1000.0
    }

    fn p99_ms(&self) -> f64 {
        self.value_at_quantile(99.0) as f64 / 1000.0
    }

    fn mean_ms(&self) -> f64 {
        self.mean() / 1000.0
    }

    fn max_ms(&self) -> f64 {
        self.max() as f64 / 1000.0
    }

    fn min_ms(&self) -> f64 {
        self.min() as f64 / 1000.0
    }
}
