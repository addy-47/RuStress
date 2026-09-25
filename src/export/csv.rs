use crate::core::result::ExperimentResult;

/// JMeter-compatible CSV column order.
const JMETER_HEADER: [&str; 17] = [
    "timeStamp",
    "elapsed",
    "label",
    "responseCode",
    "responseMessage",
    "threadName",
    "dataType",
    "success",
    "failureMessage",
    "bytes",
    "sentBytes",
    "grpThreads",
    "allThreads",
    "URL",
    "Latency",
    "IdleTime",
    "Connect",
];

/// Write results to a JMeter-compatible CSV file.
pub fn export_csv(results: &[ExperimentResult], path: &str) -> anyhow::Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.write_record(JMETER_HEADER)?;

    for r in results {
        let message = r.error.clone().unwrap_or_default();
        wtr.write_record([
            r.timestamp.timestamp_millis().to_string(),
            r.service_time.as_micros().to_string(),
            r.query.clone(),
            r.status.to_string(),
            message.clone(),
            r.user_id.clone(),
            "text".to_string(),
            r.success.to_string(),
            message,
            r.bytes.max(0).to_string(),
            "0".to_string(),
            "1".to_string(),
            "1".to_string(),
            String::new(),
            r.latency.as_micros().to_string(),
            "0".to_string(),
            "0".to_string(),
        ])?;
    }

    wtr.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::result::ExperimentResult;
    use chrono::Utc;
    use std::time::Duration;

    fn sample(status: u16, success: bool) -> ExperimentResult {
        ExperimentResult {
            timestamp: Utc::now(),
            latency: Duration::from_millis(11),
            service_time: Duration::from_millis(10),
            queue_wait: Duration::from_millis(1),
            status,
            success,
            bytes: 128,
            user_id: "user-1".into(),
            query: "custom".into(),
            error: None,
            response_body: None,
        }
    }

    #[test]
    fn export_csv_writes_jmeter_header() {
        let dir = std::env::temp_dir().join("rustress-export-csv-header");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.csv");

        export_csv(&[], path.to_str().unwrap()).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let header = contents.lines().next().unwrap();
        assert_eq!(header, JMETER_HEADER.join(","));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn export_csv_writes_one_row_per_result() {
        let dir = std::env::temp_dir().join("rustress-export-csv-rows");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.csv");

        let results = vec![sample(200, true), sample(500, false)];
        export_csv(&results, path.to_str().unwrap()).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 3, "expected header + 2 rows");
        assert!(lines[1].contains("200"));
        assert!(lines[2].contains("500"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
