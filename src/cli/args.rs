use crate::core::config::{Config, Mode};
use crate::core::constants::VERSION;
use clap::{Parser, Subcommand};

const HELP_BANNER: &str = concat!(
    "\x1b[38;2;196;248;245m██████╗ ██╗   ██╗███████╗████████╗██████╗ ███████╗███████╗\x1b[0m\n",
    "\x1b[38;2;173;226;224m██╔══██╗██║   ██║██╔════╝╚══██╔══╝██╔══██╗██╔════╝██╔════╝\x1b[0m\n",
    "\x1b[38;2;150;204;203m██████╔╝██║   ██║███████╗   ██║   ██████╔╝█████╗  ███████╗\x1b[0m\n",
    "\x1b[38;2;127;182;182m██╔══██╗██║   ██║╚════██║   ██║   ██╔══██╗██╔══╝  ╚════██║\x1b[0m\n",
    "\x1b[38;2;104;160;161m██║  ██║╚██████╔╝███████║   ██║   ██║  ██║███████╗███████║\x1b[0m\n",
    "\x1b[38;2;79;137;139m╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝   ╚═╝  ╚═╝╚══════╝╚══════╝\x1b[0m\n",
    "\x1b[38;2;196;248;245m            v",
    env!("CARGO_PKG_VERSION"),
    " | High-Performance Load Testing Engine\x1b[0m\n",
    "\n",
    "🚀 Quick Start:\n",
    "  rustress --url http://localhost:8080 --rate 100 --duration 30    # RPS mode\n",
    "  rustress --url http://localhost:8080 --users 50 --duration 30   # Users mode\n",
    "\n",
    "💡 Examples:\n",
    "  rustress                              # Launch interactive TUI\n",
    "  rustress -u http://localhost:8080     # TUI with pre-filled URL\n",
    "  rustress -u http://localhost:8080 -r 1000 -d 60  # Headless: 1000 RPS for 60s\n",
    "  rustress dummy --port 9090            # Start test server on port 9090\n",
    "  rustress report -i results.csv        # Show summary from CSV\n",
);

#[derive(Parser, Debug)]
#[command(name = "rustress")]
#[command(version = VERSION)]
#[command(about = None)]
#[command(before_help = HELP_BANNER)]
pub struct Cli {
    /// Target URL
    #[arg(short, long)]
    pub url: Option<String>,

    /// HTTP method
    #[arg(short, long)]
    pub method: Option<String>,

    /// Request body or @file.json
    #[arg(short, long)]
    pub body: Option<String>,

    /// Target RPS (open loop)
    #[arg(short, long)]
    pub rate: Option<u32>,

    /// Number of virtual users (closed loop)
    #[arg(short = 'n', long)]
    pub users: Option<u32>,

    /// Test duration in seconds
    #[arg(short, long)]
    pub duration: Option<u64>,

    /// Ramp-up duration in seconds
    #[arg(long)]
    pub ramp_up: Option<u64>,

    /// Ramp-down duration in seconds
    #[arg(long)]
    pub ramp_down: Option<u64>,

    /// Request timeout in seconds
    #[arg(long)]
    pub timeout: Option<u64>,

    /// HTTP header (Key: Value), repeatable
    #[arg(short = 'H', long, action = clap::ArgAction::Append)]
    pub header: Vec<String>,

    /// Output file prefix for reports
    #[arg(long)]
    pub out: Option<String>,

    /// Config file path (TOML)
    #[arg(long)]
    pub config: Option<String>,

    /// Subcommands
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start built-in test HTTP server
    Dummy {
        /// Port to listen on
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
    /// Export report from existing CSV/JSON data
    Report {
        /// Input file path
        #[arg(short, long)]
        input: String,
    },
}

impl Cli {
    /// Load config from file if specified, then override with CLI flags.
    pub fn into_config(&self) -> Config {
        // Start with defaults or load from file
        let mut cfg = match self.config {
            Some(ref path) => match load_config_file(path) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            },
            None => Config::default(),
        };

        // CLI flags override file config
        if let Some(ref url) = self.url {
            cfg.url = url.clone();
        }

        // Only override what the user actually passed. Assigning these
        // unconditionally erased the config file's body and output prefix, so
        // `--config load.toml` silently sent bodyless requests and exported
        // nothing while reporting a clean run.
        if let Some(ref method) = self.method {
            cfg.method = method.clone();
        }
        if let Some(ref body) = self.body {
            cfg.body = Some(body.clone());
        }
        if let Some(ref out) = self.out {
            cfg.out_prefix = Some(out.clone());
        }

        if let Some(rate) = self.rate {
            cfg.target_rps = rate;
        }
        if let Some(users) = self.users {
            cfg.num_users = users;
            cfg.mode = Mode::Users;
        }
        if let Some(dur) = self.duration {
            cfg.steady_dur_secs = dur;
        }
        if let Some(ramp) = self.ramp_up {
            cfg.ramp_up_secs = ramp;
        }
        if let Some(ramp) = self.ramp_down {
            cfg.ramp_down_secs = ramp;
        }
        if let Some(timeout) = self.timeout {
            cfg.timeout_secs = timeout;
        }

        // Parse headers
        for h in &self.header {
            if let Some((key, value)) = h.split_once(':') {
                cfg.headers
                    .insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        cfg
    }
}

/// Load a Config from a TOML file the user named explicitly.
///
/// A missing or malformed file is an error, not a warning. Falling back to
/// defaults produces an empty `url`, which silently routes the run into the
/// interactive TUI instead of reporting the typo.
fn load_config_file(path: &str) -> anyhow::Result<Config> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read config file '{path}': {e}"))?;
    toml::from_str::<Config>(&contents)
        .map_err(|e| anyhow::anyhow!("failed to parse config file '{path}': {e}"))
}
