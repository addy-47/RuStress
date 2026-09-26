//! RuStress — a high-performance load testing engine with an interactive terminal dashboard.
//!
//! The crate is published as both a library and a binary. The library exposes the
//! full load-generation stack so it can be embedded or driven programmatically;
//! the `rustress` binary is a thin CLI shell over it.
//!
//! # Subsystems
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`core`] | Domain types, configuration, and the error boundary |
//! | [`metrics`] | Lock-free counters and HDR latency histograms |
//! | [`templates`] | Per-request value injection and file-backed data |
//! | [`runner`] | Open-loop (RPS) and closed-loop (Users) load scheduling |
//! | [`tui`] | Interactive ratatui dashboard |
//! | [`export`] | CSV / JSON / summary report writers |
//! | [`dummy`] | Built-in target HTTP server for self-testing |
//! | [`cli`] | Argument definitions and subcommand dispatch |
//!
//! # Example
//!
//! ```no_run
//! use rustress::core::Config;
//! use rustress::runner::LoadEngine;
//! use tokio::sync::mpsc;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let cfg = Config {
//!     url: "http://127.0.0.1:8080/fast".into(),
//!     target_rps: 200,
//!     steady_dur_secs: 10,
//!     ..Default::default()
//! };
//! let (tx, _rx) = mpsc::channel(rustress::core::constants::STATS_CHANNEL_CAPACITY);
//! let engine = LoadEngine::new(cfg, tx)?;
//! engine.run(tokio_util::sync::CancellationToken::new()).await;
//! # Ok(())
//! # }
//! ```

pub mod cli;
pub mod core;
pub mod dummy;
pub mod export;
pub mod metrics;
pub mod runner;
pub mod templates;
pub mod tui;

pub use core::config::{Config, Mode};
pub use core::result::ExperimentResult;
pub use core::snapshot::StatsSnapshot;
pub use runner::LoadEngine;
