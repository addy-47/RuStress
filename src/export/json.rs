use crate::core::result::ExperimentResult;

/// Write the full result set to a pretty-printed JSON file.
pub fn export_json(results: &[ExperimentResult], path: &str) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(results)?;
    std::fs::write(path, json.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::result::ExperimentResult;
    use chrono::Utc;
    use std::time::Duration;

    fn sample() -> ExperimentResult {
        ExperimentResult {
            timestamp: Utc::now(),
            latency: Duration::from_millis(11),
            service_time: Duration::from_millis(10),
            queue_wait: Duration::from_millis(1),
            status: 200,
            success: true,
            bytes: 128,
            user_id: "user-1".into(),
            query: "custom".into(),
            error: None,
            response_body: None,
        }
    }

    #[test]
    fn export_json_round_trips() {
        let dir = std::env::temp_dir().join("rustress-export-json");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.json");

        let results = vec![sample()];
        export_json(&results, path.to_str().unwrap()).unwrap();

        let decoded: Vec<ExperimentResult> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].status, 200);
        std::fs::remove_dir_all(&dir).ok();
    }
}
