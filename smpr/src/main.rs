mod config;
mod detection;
mod rating;
mod report;
mod server;
mod tui;

use clap::{Parser, Subcommand};
use std::process;

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
        /// Scope to a specific music library
        #[arg(long)]
        library: Option<String>,

        /// Scope to a location within a library
        #[arg(long)]
        location: Option<String>,

        /// Target a named server (repeatable)
        #[arg(long)]
        server: Option<Vec<String>>,

        /// Analyze only — no server updates
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// CSV report output path
        #[arg(long)]
        report: Option<String>,

        /// Re-evaluate tracks that already have a rating
        #[arg(long)]
        overwrite: bool,

        /// Skip tracks that already have any rating
        #[arg(long)]
        skip_existing: bool,

        /// Ignore per-library force_rating from config
        #[arg(long)]
        ignore_forced: bool,

        /// Path to TOML config file
        #[arg(long)]
        config: Option<String>,

        /// Path to .env file
        #[arg(long)]
        env_file: Option<String>,

        /// Server URL for one-off use (requires --api-key)
        #[arg(long)]
        server_url: Option<String>,

        /// API key for one-off use (requires --server-url)
        #[arg(long)]
        api_key: Option<String>,

        /// Debug logging
        #[arg(short, long)]
        verbose: bool,
    },

    /// Set a fixed rating on all tracks in scope (no lyrics evaluation)
    Force {
        /// Rating to set (e.g. G, PG-13, R)
        rating: String,

        #[arg(long)]
        library: Option<String>,
        #[arg(long)]
        location: Option<String>,
        #[arg(long)]
        server: Option<Vec<String>>,
        #[arg(short = 'n', long)]
        dry_run: bool,
        #[arg(long)]
        report: Option<String>,
        #[arg(long)]
        overwrite: bool,
        #[arg(long)]
        skip_existing: bool,
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        env_file: Option<String>,
        #[arg(long)]
        server_url: Option<String>,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(short, long)]
        verbose: bool,
    },

    /// Remove OfficialRating from all tracks in scope
    Reset {
        #[arg(long)]
        library: Option<String>,
        #[arg(long)]
        location: Option<String>,
        #[arg(long)]
        server: Option<Vec<String>>,
        #[arg(short = 'n', long)]
        dry_run: bool,
        #[arg(long)]
        report: Option<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        env_file: Option<String>,
        #[arg(long)]
        server_url: Option<String>,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(short, long)]
        verbose: bool,
    },

    /// Interactive setup wizard for server connection and config
    Configure {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        env_file: Option<String>,
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
