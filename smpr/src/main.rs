mod config;
mod detection;
mod rating;
mod report;
mod server;
mod tui;
mod util;

use clap::{Args, Parser, Subcommand};
use log::LevelFilter;
use std::path::PathBuf;
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
struct OverwriteOpts {
    /// Re-evaluate tracks that already have a rating (default unless changed in config)
    #[arg(long, conflicts_with = "skip_existing")]
    overwrite: bool,

    /// Skip tracks that already have any rating (overrides config default)
    #[arg(long, conflicts_with = "overwrite")]
    skip_existing: bool,
}

impl OverwriteOpts {
    /// Resolve to Option<bool>: Some(true)=overwrite, Some(false)=skip, None=use config default.
    fn resolve(&self) -> Option<bool> {
        if self.overwrite {
            Some(true)
        } else if self.skip_existing {
            Some(false)
        } else {
            None
        }
    }
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

/// Build a CliInput from CommonOpts + optional overwrite/ignore_forced flags.
fn build_cli_input(
    common: &CommonOpts,
    overwrite: Option<bool>,
    ignore_forced: bool,
) -> config::CliInput {
    config::CliInput {
        config_path: common.config.as_ref().map(PathBuf::from),
        env_file: common.env_file.as_ref().map(PathBuf::from),
        server_url: common.server_url.clone(),
        api_key: common.api_key.clone(),
        server_filter: common.server.clone(),
        overwrite,
        dry_run: common.dry_run,
        report: common.report.clone(),
        library: common.library.clone(),
        location: common.location.clone(),
        verbose: common.verbose,
        ignore_forced,
    }
}

fn load_config(
    common: &CommonOpts,
    overwrite: Option<bool>,
    ignore_forced: bool,
) -> config::Config {
    let cli_input = build_cli_input(common, overwrite, ignore_forced);
    config::Config::load_from_paths(&cli_input).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        process::exit(1);
    })
}

fn main() {
    let cli = Cli::parse();

    // Determine verbose from any subcommand before initializing logger
    let verbose = match &cli.command {
        Commands::Rate { common, .. }
        | Commands::Force { common, .. }
        | Commands::Reset { common } => common.verbose,
        Commands::Configure { verbose, .. } => *verbose,
    };

    env_logger::Builder::new()
        .filter_level(if verbose {
            LevelFilter::Debug
        } else {
            LevelFilter::Warn
        })
        .format_target(false)
        .format_timestamp(None)
        .init();

    match cli.command {
        Commands::Rate {
            common,
            overwrite,
            ignore_forced,
        } => {
            let cfg = load_config(&common, overwrite.resolve(), ignore_forced);
            if cfg.verbose {
                eprintln!("Config loaded: {} server(s)", cfg.servers.len());
            }
            eprintln!("rate: not yet implemented");
            process::exit(1);
        }
        Commands::Force {
            rating,
            common,
            overwrite,
        } => {
            let cfg = load_config(&common, overwrite.resolve(), false);
            if cfg.verbose {
                eprintln!(
                    "Config loaded: {} server(s), force rating={rating}",
                    cfg.servers.len()
                );
            }
            eprintln!("force: not yet implemented");
            process::exit(1);
        }
        Commands::Reset { common } => {
            let cfg = load_config(&common, None, false);
            if cfg.verbose {
                eprintln!("Config loaded: {} server(s)", cfg.servers.len());
            }
            eprintln!("reset: not yet implemented");
            process::exit(1);
        }
        Commands::Configure { .. } => {
            eprintln!("configure: not yet implemented");
            process::exit(1);
        }
    }
}
