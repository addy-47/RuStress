use crate::core::config::Config;
use crate::runner::LoadEngine;
use tokio::sync::mpsc;

/// Run in interactive TUI mode.
pub async fn run_tui(cfg: Config) -> anyhow::Result<()> {
    // No banner in TUI mode — the TUI itself is the interface
    let (tx, rx) = mpsc::unbounded_channel();
    let engine = LoadEngine::new(cfg.clone(), tx)?;

    // Run TUI
    crate::tui::run_tui(cfg, rx).await?;

    // After TUI exits, print summary if there are results
    let results = engine.stats().get_results();
    if !results.is_empty() {
        print_summary(&results);
    }

    Ok(())
}

fn print_summary(results: &[crate::core::result::ExperimentResult]) {
    let total = results.len();
    let success = results.iter().filter(|r| r.success).count();
    let fail = total - success;

    println!("\n{}", "═".repeat(60));
    println!("  SUMMARY");
    println!("{}", "═".repeat(60));
    println!("  Total:     {}", total);
    println!("  Success:   {}", success);
    println!("  Failed:    {}", fail);
    if total > 0 {
        println!("  Success:   {:.1}%", success as f64 / total as f64 * 100.0);
    }
    println!("{}", "═".repeat(60));
}
