//! ============================================================================
//! file_body.rs — POST a payload read from disk
//! ============================================================================
//! Category     : Utility Tool
//! Component    : runner::request::PreparedRequest (`@file` body loading)
//! Prerequisites: none (writes its own payload to a temp file)
//! Execution    : cargo run --release --example file_body
//! Metrics      : request count and bytes sent
//!
//! A body of the form `@path` is read from disk once, at construction, not once
//! per request. A multi-megabyte payload is the case this exists for: reading
//! it inside the request path would put the file size in the per-request cost
//! and re-read identical bytes for every request in the run.
//!
//! The temp file is deliberately left in place on exit and its path printed, so
//! the payload can be inspected afterwards.
//! ============================================================================

#[path = "common/mod.rs"]
mod shared;

use std::io::Write;

use rustress::core::Config;
use shared::{report, run_to_completion, start_target};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = start_target().await?;
    println!("target: {base}/fast");

    let path = std::env::temp_dir().join("rustress_example_payload.json");
    let mut file = std::fs::File::create(&path)?;
    write!(file, "{{\"payload\": \"{}\"}}", "x".repeat(4096))?;
    drop(file);
    println!("payload file: {}", path.display());

    let cfg = Config {
        url: format!("{base}/fast"),
        method: "POST".to_string(),
        body: Some(format!("@{}", path.display())),
        target_rps: 100,
        steady_dur_secs: 3,
        timeout_secs: 10,
        ..Default::default()
    };

    report(
        "100 RPS POSTing a 4 KB file body",
        &run_to_completion(cfg).await?,
    );
    Ok(())
}
