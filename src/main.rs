use anyhow::Result;
use clap::Parser;
use rustress::cli::args::{Cli, Commands};
use rustress::cli::commands;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Dummy { port }) => {
            commands::dummy::run(port).await?;
        }
        Some(Commands::Report { input }) => {
            commands::report::run(&input)?;
        }
        None => {
            let cfg = cli.into_config();

            if cfg.url.is_empty() && cfg.command.is_none() {
                commands::tui_mode::run_tui(cfg).await?;
            } else {
                commands::headless::run_headless(cfg).await?;
            }
        }
    }

    Ok(())
}
