use crate::metrics::histogram::LatencyHistogram;

/// Percentile extraction helper for latency histograms.
///
/// All methods return values in **milliseconds** (the histogram stores
/// microseconds).
///
/// # Quantile scale
///
/// `hdrhistogram::Histogram::value_at_quantile` takes a **fraction in the
/// range 0.0–1.0**, not a percentage. Passing `50.0` instead of `0.50` clamps
/// to the 100th percentile, which silently makes every reported percentile
/// equal the maximum observed latency. The conversion is done here, once, so
/// no call site can get it wrong.
pub trait PercentileExt {
    fn p50_ms(&self) -> f64;
    fn p90_ms(&self) -> f64;
    fn p95_ms(&self) -> f64;
    fn p99_ms(&self) -> f64;
    fn p999_ms(&self) -> f64;
    fn mean_ms(&self) -> f64;
    fn max_ms(&self) -> f64;
    fn min_ms(&self) -> f64;
}

impl PercentileExt for LatencyHistogram {
    fn p50_ms(&self) -> f64 {
        self.value_at_quantile(0.50) as f64 / 1000.0
    }

    fn p90_ms(&self) -> f64 {
        self.value_at_quantile(0.90) as f64 / 1000.0
    }

    fn p95_ms(&self) -> f64 {
        self.value_at_quantile(0.95) as f64 / 1000.0
    }

    fn p99_ms(&self) -> f64 {
        self.value_at_quantile(0.99) as f64 / 1000.0
    }

    fn p999_ms(&self) -> f64 {
        self.value_at_quantile(0.999) as f64 / 1000.0
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a histogram from a uniform millisecond spread.
    fn uniform(low_ms: u64, high_ms: u64, per_bucket: u64) -> LatencyHistogram {
        let hist = LatencyHistogram::new();
        for ms in low_ms..=high_ms {
            for _ in 0..per_bucket {
                hist.record(ms * 1000);
            }
        }
        hist
    }

    #[test]
    fn empty_histogram_reports_zero_percentiles() {
        let hist = LatencyHistogram::new();
        assert_eq!(hist.p50_ms(), 0.0);
        assert_eq!(hist.p99_ms(), 0.0);
    }

    #[test]
    fn single_sample_reports_that_value_for_every_percentile() {
        // 3-significant-figure HDR buckets quantize slightly above the input,
        // so assert within one bucket width rather than exactly.
        let hist = LatencyHistogram::new();
        hist.record(5_000);
        let expected = hist.max_ms();
        assert_eq!(hist.p50_ms(), expected);
        assert_eq!(hist.p90_ms(), expected);
        assert_eq!(hist.p99_ms(), expected);
        assert!((expected - 5.0).abs() < 0.01, "max was {expected}");
    }

    /// The regression guard for the quantile-scale bug.
    ///
    /// Passing a percentage where a fraction is expected clamps every
    /// percentile to the maximum, making p50 == p99 == max. A real spread must
    /// produce strictly increasing percentiles.
    #[test]
    fn percentiles_are_distinct_on_a_spread_distribution() {
        let hist = uniform(10, 50, 20);

        let p50 = hist.p50_ms();
        let p90 = hist.p90_ms();
        let p95 = hist.p95_ms();
        let p99 = hist.p99_ms();

        assert!(
            p50 < p90,
            "p50 ({p50}) must be below p90 ({p90}); equal values mean the quantile scale is wrong"
        );
        assert!(p90 < p95, "p90 ({p90}) must be below p95 ({p95})");
        assert!(p95 < p99, "p95 ({p95}) must be below p99 ({p99})");
        assert!(
            p99 <= hist.max_ms(),
            "p99 ({p99}) cannot exceed max ({})",
            hist.max_ms()
        );
    }

    #[test]
    fn percentiles_land_near_the_analytic_values() {
        // Uniform 10-50ms => median ~30ms, p90 ~46ms, p99 ~50ms.
        let hist = uniform(10, 50, 20);

        assert!(
            (hist.p50_ms() - 30.0).abs() < 1.5,
            "p50 was {}",
            hist.p50_ms()
        );
        assert!(
            (hist.p90_ms() - 46.0).abs() < 2.0,
            "p90 was {}",
            hist.p90_ms()
        );
        assert!(
            (hist.p99_ms() - 50.0).abs() < 1.5,
            "p99 was {}",
            hist.p99_ms()
        );
    }

    #[test]
    fn a_small_tail_of_outliers_does_not_move_p50() {
        // 4100 base samples against 50 outliers: the median stays inside the
        // base distribution, which is the property that makes p50 useful.
        let hist = uniform(10, 50, 100);
        let p50_before = hist.p50_ms();
        for _ in 0..50 {
            hist.record(9_000_000);
        }
        assert!(
            (hist.p50_ms() - p50_before).abs() < 0.5,
            "p50 must be robust: {} -> {}",
            p50_before,
            hist.p50_ms()
        );
        assert!(
            hist.p99_ms() > 1_000.0,
            "p99 must reflect the outliers, got {}",
            hist.p99_ms()
        );
    }

    #[test]
    fn mean_tracks_the_actual_average() {
        let hist = uniform(10, 50, 20);
        assert!(
            (hist.mean_ms() - 30.0).abs() < 1.0,
            "mean was {}",
            hist.mean_ms()
        );
    }

    #[test]
    fn min_and_max_bound_every_percentile() {
        let hist = uniform(10, 50, 20);
        let lo = hist.min_ms();
        let hi = hist.max_ms();
        for p in [hist.p50_ms(), hist.p90_ms(), hist.p95_ms(), hist.p99_ms()] {
            assert!(p >= lo, "percentile {p} below min {lo}");
            assert!(p <= hi, "percentile {p} above max {hi}");
        }
    }
}
