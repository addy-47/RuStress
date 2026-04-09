//! Report generation — CSV, JSON, and summary exports.

use rustress_core::result::ExperimentResult;

/// Export results to CSV.
pub fn export_csv(results: &[ExperimentResult], path: &str) -> anyhow::Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;

    // JMeter-compatible header
    wtr.write_record(&[
        "timeStamp", "elapsed", "label", "responseCode", "responseMessage",
        "threadName", "dataType", "success", "failureMessage", "bytes",
        "sentBytes", "grpThreads", "allThreads", "URL", "Latency", "IdleTime", "Connect",
    ])?;

    for r in results {
        wtr.write_record(&[
            format!("{}", r.timestamp.timestamp_millis()),
            format!("{}", r.service_time.as_micros()),
            r.query.clone(),
            format!("{}", r.status),
            r.error.clone().unwrap_or_default(),
            r.user_id.clone(),
            "text".to_string(),
            format!("{}", r.success),
            r.error.clone().unwrap_or_default(),
            format!("{}", r.bytes.max(0)),
            "0".to_string(),
            "1".to_string(),
            "1".to_string(),
            "".to_string(),
            format!("{}", r.latency.as_micros()),
            "0".to_string(),
            "0".to_string(),
        ])?;
    }

    wtr.flush()?;
    Ok(())
}

/// Export results to JSON.
pub fn export_json(results: &[ExperimentResult], path: &str) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(results)?;
    std::fs::write(path, json.as_bytes())?;
    Ok(())
}

/// Export a summary JSON.
pub fn export_summary(results: &[ExperimentResult], path: &str) -> anyhow::Result<()> {
    let total = results.len() as u64;
    let success = results.iter().filter(|r| r.success).count() as u64;
    let fail = total - success;

    let mut latencies: Vec<u64> = results.iter().map(|r| r.service_time.as_micros() as u64).collect();
    latencies.sort();

    let p50 = percentile(&latencies, 50.0);
    let p90 = percentile(&latencies, 90.0);
    let p95 = percentile(&latencies, 95.0);
    let p99 = percentile(&latencies, 99.0);

    let summary = serde_json::json!({
        "total_requests": total,
        "total_success": success,
        "total_fail": fail,
        "p50_us": p50,
        "p90_us": p90,
        "p95_us": p95,
        "p99_us": p99,
    });

    let json = serde_json::to_string_pretty(&summary)?;
    std::fs::write(path, json.as_bytes())?;
    Ok(())
}

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
