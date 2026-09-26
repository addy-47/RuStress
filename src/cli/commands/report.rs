use std::path::Path;

/// Run the report export command.
///
/// Reads a CSV or JSON results file and prints a summary.
pub fn run(input: &str) -> anyhow::Result<()> {
    let path = Path::new(input);
    if !path.exists() {
        anyhow::bail!("Input file not found: {}", input);
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "csv" => report_from_csv(input)?,
        "json" => report_from_json(input)?,
        _ => anyhow::bail!("Unsupported file format: {}. Use .csv or .json", ext),
    }

    Ok(())
}

fn report_from_csv(path: &str) -> anyhow::Result<()> {
    let mut rdr = csv::Reader::from_path(path)?;
    let mut total: u64 = 0;
    let mut success: u64 = 0;
    let mut fail: u64 = 0;
    let mut latencies = Vec::new();
    let mut status_codes = std::collections::HashMap::new();
    let mut error_counts = std::collections::HashMap::new();

    for result in rdr.records() {
        let record = result?;
        total += 1;

        let elapsed_us: u64 = record.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let status: u16 = record.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let success_flag = record
            .get(7)
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);
        let error = record.get(8).and_then(|v| {
            let s = v.trim();
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        });

        if success_flag {
            success += 1;
        } else {
            fail += 1;
        }

        latencies.push(elapsed_us);
        *status_codes.entry(status).or_insert(0) += 1;
        if let Some(err) = error {
            *error_counts.entry(err).or_insert(0) += 1;
        }
    }

    latencies.sort();
    print_summary(
        total,
        success,
        fail,
        &latencies,
        &status_codes,
        &error_counts,
    );
    Ok(())
}

fn report_from_json(path: &str) -> anyhow::Result<()> {
    let data = std::fs::read_to_string(path)?;
    let results: Vec<crate::core::result::ExperimentResult> = serde_json::from_str(&data)?;

    let total = results.len() as u64;
    let success = results.iter().filter(|r| r.success).count() as u64;
    let fail = total - success;

    let mut latencies: Vec<u64> = results
        .iter()
        .map(|r| r.service_time.as_micros() as u64)
        .collect();
    latencies.sort();

    let mut status_codes = std::collections::HashMap::new();
    let mut error_counts = std::collections::HashMap::new();
    for r in &results {
        *status_codes.entry(r.status).or_insert(0) += 1;
        if let Some(ref err) = r.error {
            if !err.is_empty() {
                *error_counts.entry(err.clone()).or_insert(0) += 1;
            }
        }
    }

    print_summary(
        total,
        success,
        fail,
        &latencies,
        &status_codes,
        &error_counts,
    );
    Ok(())
}

fn print_summary(
    total: u64,
    success: u64,
    fail: u64,
    latencies: &[u64],
    status_codes: &std::collections::HashMap<u16, u64>,
    error_counts: &std::collections::HashMap<String, u64>,
) {
    println!();
    println!("{}", "═".repeat(60));
    println!("  REPORT SUMMARY");
    println!("{}", "═".repeat(60));
    println!("  Total Requests: {}", total);
    println!("  Success:        {}", success);
    println!("  Failed:         {}", fail);
    if total > 0 {
        println!(
            "  Success Rate:   {:.1}%",
            success as f64 / total as f64 * 100.0
        );
    }

    if !latencies.is_empty() {
        println!();
        println!("  Latencies (µs):");
        println!("    P50: {}", percentile_us(latencies, 50.0));
        println!("    P90: {}", percentile_us(latencies, 90.0));
        println!("    P95: {}", percentile_us(latencies, 95.0));
        println!("    P99: {}", percentile_us(latencies, 99.0));
        println!("    Max: {}", latencies.last().copied().unwrap_or(0));
    }

    if !status_codes.is_empty() {
        println!();
        println!("  Status Codes:");
        let mut codes: Vec<_> = status_codes.iter().collect();
        codes.sort_by_key(|(code, _)| **code);
        for (code, count) in codes {
            println!("    {}  {}", code, count);
        }
    }

    if !error_counts.is_empty() {
        println!();
        println!("  Errors:");
        for (err, count) in error_counts {
            println!("    {:>6} {}", count, err);
        }
    }

    println!("{}", "═".repeat(60));
}

fn percentile_us(sorted: &[u64], pct: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}
