use crate::core::result::ExperimentResult;
use serde_json::json;

/// Write aggregate percentiles and totals to a summary JSON file.
pub fn export_summary(results: &[ExperimentResult], path: &str) -> anyhow::Result<()> {
    let total = results.len() as u64;
    let success = results.iter().filter(|r| r.success).count() as u64;
    let fail = total.saturating_sub(success);

    let mut latencies: Vec<u64> = results
        .iter()
        .map(|r| r.service_time.as_micros() as u64)
        .collect();
    latencies.sort_unstable();

    let summary = json!({
        "total_requests": total,
        "total_success": success,
        "total_fail": fail,
        "p50_us": percentile(&latencies, 50.0),
        "p90_us": percentile(&latencies, 90.0),
        "p95_us": percentile(&latencies, 95.0),
        "p99_us": percentile(&latencies, 99.0),
    });

    std::fs::write(path, serde_json::to_string_pretty(&summary)?)?;
    Ok(())
}

/// Nearest-rank percentile over an ascending-sorted slice.
fn percentile(sorted: &[u64], pct: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_empty() {
        assert_eq!(percentile(&[], 50.0), 0);
    }

    #[test]
    fn test_percentile_single() {
        let data = &[100];
        assert_eq!(percentile(data, 50.0), 100);
        assert_eq!(percentile(data, 99.0), 100);
    }

    #[test]
    fn test_percentile_values() {
        let data: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&data, 50.0), 51);
        assert_eq!(percentile(&data, 90.0), 90);
        assert_eq!(percentile(&data, 99.0), 99);
    }
}
