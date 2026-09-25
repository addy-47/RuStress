mod cli;
mod commands;

use anyhow::Result;
use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(cli::Commands::Dummy { port }) => {
            commands::dummy::run(port).await?;
        }
        Some(cli::Commands::Report { input }) => {
            commands::report::run(&input)?;
        }
        None => {
            let cfg = cli.into_config();

            if cfg.url.is_empty() && cfg.command.is_none() {
                // No URL = interactive TUI mode
                commands::tui_mode::run_tui(cfg).await?;
            } else {
                // URL provided = headless mode
                commands::headless::run_headless(cfg).await?;
            }
        }
    }

    Ok(())
}
