use crate::core::snapshot::StatsSnapshot;
use serde_json::json;

/// Write aggregate totals and latency percentiles to a summary JSON file.
///
/// Percentiles come from the run's `StatsSnapshot`, which is computed from the
/// HDR histogram over **every** request. Recomputing them from the retention
/// ring instead would emit a second, different set of numbers for any run
/// longer than the ring, with nothing marking the file as partial. The
/// retention counts are included so the artifact states what it is.
pub fn export_summary(results_len: usize, snap: &StatsSnapshot, path: &str) -> anyhow::Result<()> {
    let summary = json!({
        "total_requests": snap.requests,
        "total_success": snap.success,
        "total_fail": snap.fail,
        "dropped_scheduled": snap.dropped_scheduled,
        "results_retained": results_len,
        "results_evicted_from_ring": snap.dropped_results,
        "measurement_valid": snap.dropped_scheduled == 0,
        "p50_us": (snap.p50_service_ms * 1000.0) as u64,
        "p90_us": (snap.p90_service_ms * 1000.0) as u64,
        "p95_us": (snap.p95_service_ms * 1000.0) as u64,
        "p99_us": (snap.p99_service_ms * 1000.0) as u64,
        "max_us": (snap.max_service_ms * 1000.0) as u64,
    });

    std::fs::write(path, serde_json::to_string_pretty(&summary)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_reports_authoritative_counts_not_ring_length() {
        let dir = std::env::temp_dir().join("rustress-export-summary");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("summary.json");

        let mut snap = StatsSnapshot {
            requests: 3_000_000,
            success: 2_900_000,
            fail: 100_000,
            p50_service_ms: 12.5,
            p99_service_ms: 480.0,
            ..Default::default()
        };
        snap.dropped_results = 2_950_000;

        export_summary(50_000, &snap, path.to_str().unwrap()).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["total_requests"], 3_000_000);
        assert_eq!(v["p50_us"], 12_500);
        assert_eq!(v["p99_us"], 480_000);
        assert_eq!(v["results_evicted_from_ring"], 2_950_000);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn summary_marks_a_shed_run_as_invalid() {
        let dir = std::env::temp_dir().join("rustress-export-summary-invalid");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("summary.json");

        let snap = StatsSnapshot {
            requests: 100,
            dropped_scheduled: 42,
            ..Default::default()
        };
        export_summary(100, &snap, path.to_str().unwrap()).unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["measurement_valid"], false);
        assert_eq!(v["dropped_scheduled"], 42);
        std::fs::remove_dir_all(&dir).ok();
    }
}
