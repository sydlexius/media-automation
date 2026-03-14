mod config;
mod detection;
mod rating;
mod report;
mod server;
mod tui;

use clap::{Args, Parser, Subcommand};
use std::process;

/// Options shared across rate, force, and reset subcommands.
#[derive(Args, Clone)]
struct CommonOpts {
    /// Scope to a specific music library
    #[arg(long)]
    library: Option<String>,

    /// Scope to a location within a library
    #[arg(long)]
    location: Option<String>,

    /// Target a named server (repeatable)
    #[arg(long)]
    server: Option<Vec<String>>,

    /// Analyze only -- no server updates
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// CSV report output path
    #[arg(long)]
    report: Option<String>,

    /// Path to TOML config file
    #[arg(long)]
    config: Option<String>,

    /// Path to .env file
    #[arg(long)]
    env_file: Option<String>,

    /// Server URL for one-off use (requires --api-key)
    #[arg(long, requires = "api_key")]
    server_url: Option<String>,

    /// API key for one-off use (requires --server-url)
    #[arg(long, requires = "server_url")]
    api_key: Option<String>,

    /// Debug logging
    #[arg(short, long)]
    verbose: bool,
}

/// Overwrite/skip behavior for rate and force subcommands.
#[derive(Args, Clone)]
#[group(multiple = false)]
struct OverwriteOpts {
    /// Re-evaluate tracks that already have a rating (default unless changed in config)
    #[arg(long)]
    overwrite: bool,

    /// Skip tracks that already have any rating (overrides config default)
    #[arg(long)]
    skip_existing: bool,
}

#[derive(Parser)]
#[command(
    name = "smpr",
    about = "Fetch lyrics from Emby/Jellyfin, detect explicit content, set parental ratings",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch lyrics, detect explicit content, set ratings
    Rate {
        #[command(flatten)]
        common: CommonOpts,

        #[command(flatten)]
        overwrite: OverwriteOpts,

        /// Ignore per-library force_rating from config; evaluate lyrics normally
        #[arg(long)]
        ignore_forced: bool,
    },

    /// Set a fixed rating on all tracks in scope (no lyrics evaluation)
    Force {
        /// Rating to set (e.g. G, PG-13, R)
        rating: String,

        #[command(flatten)]
        common: CommonOpts,

        #[command(flatten)]
        overwrite: OverwriteOpts,
    },

    /// Remove OfficialRating from all tracks in scope
    Reset {
        #[command(flatten)]
        common: CommonOpts,
    },

    /// Interactive setup wizard for server connection and config
    Configure {
        /// Path to TOML config file
        #[arg(long)]
        config: Option<String>,

        /// Path to .env file
        #[arg(long)]
        env_file: Option<String>,

        /// Debug logging
        #[arg(short, long)]
        verbose: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Rate { .. } => {
            eprintln!("rate: not yet implemented");
            process::exit(1);
        }
        Commands::Force { .. } => {
            eprintln!("force: not yet implemented");
            process::exit(1);
        }
        Commands::Reset { .. } => {
            eprintln!("reset: not yet implemented");
            process::exit(1);
        }
        Commands::Configure { .. } => {
            eprintln!("configure: not yet implemented");
            process::exit(1);
        }
    }
}
