#![allow(dead_code)]

mod auth;
mod detection;
mod library;
mod output;
mod preferences;
mod server;

use crate::config;
use std::path::PathBuf;

/// Errors that can occur during the wizard.
#[derive(Debug)]
pub enum WizardError {
    /// Server URL not responding.
    ServerUnreachable(String),
    /// Authentication failed (with context).
    AuthFailed(String),
    /// IO error writing config/env files.
    Io(std::io::Error),
    /// TOML serialization failure.
    Serialization(toml::ser::Error),
    /// User cancelled (Ctrl+C / Escape).
    UserCancelled,
    /// inquire prompt error.
    Prompt(String),
}

impl std::fmt::Display for WizardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServerUnreachable(msg) => write!(f, "server unreachable: {msg}"),
            Self::AuthFailed(msg) => write!(f, "authentication failed: {msg}"),
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Serialization(e) => write!(f, "config serialization error: {e}"),
            Self::UserCancelled => write!(f, "wizard cancelled"),
            Self::Prompt(msg) => write!(f, "prompt error: {msg}"),
        }
    }
}

impl std::error::Error for WizardError {}

impl From<std::io::Error> for WizardError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<toml::ser::Error> for WizardError {
    fn from(e: toml::ser::Error) -> Self {
        Self::Serialization(e)
    }
}

/// Convert an inquire::InquireError to WizardError.
fn from_inquire(e: inquire::InquireError) -> WizardError {
    match e {
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
            WizardError::UserCancelled
        }
        other => WizardError::Prompt(other.to_string()),
    }
}

/// Resolve the config directory for reading/writing.
///
/// Priority:
/// 1. --config flag → parent directory of that path
/// 2. ./explicit_config.toml exists in CWD → CWD
/// 3. dirs::config_dir()/smpr/ → platform default
pub fn resolve_config_dir(cli_config: Option<&str>) -> PathBuf {
    if let Some(path) = cli_config {
        let p = PathBuf::from(path);
        let parent = p.parent().unwrap_or(&p);
        if parent.as_os_str().is_empty() {
            return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        }
        return parent.to_path_buf();
    }

    let cwd_config = std::env::current_dir()
        .ok()
        .map(|d| d.join("explicit_config.toml"));
    if let Some(ref p) = cwd_config
        && p.exists()
    {
        return p.parent().unwrap().to_path_buf();
    }

    dirs::config_dir()
        .map(|d| d.join("smpr"))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Main entry point for the configure wizard.
pub fn run_wizard(
    cli_config: Option<&str>,
    cli_env_file: Option<&str>,
    verbose: bool,
) -> Result<(), WizardError> {
    let config_dir = resolve_config_dir(cli_config);
    let config_filename = if let Some(cfg) = cli_config {
        let cfg_path = PathBuf::from(cfg);
        // Reject directories
        if cfg_path.is_dir() {
            return Err(WizardError::Prompt(format!(
                "--config must be a file path, not a directory: {}",
                cfg_path.display()
            )));
        }
        match cfg_path.file_name() {
            Some(name) => name.to_string_lossy().to_string(),
            None => {
                return Err(WizardError::Prompt(format!(
                    "invalid --config path (expected a file path): {}",
                    cfg_path.display()
                )));
            }
        }
    } else if config_dir == std::env::current_dir().unwrap_or_default() {
        "explicit_config.toml".to_string()
    } else {
        "config.toml".to_string()
    };
    let config_path = config_dir.join(&config_filename);

    let env_path = match cli_env_file {
        Some(p) => PathBuf::from(p), // CLI-provided path: relative to CWD (like other subcommands)
        None => config_dir.join(".env"), // Default: alongside config file
    };

    // Step 0: Detect existing config
    let existing = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        match config::parse_toml(&content) {
            Ok(raw) => Some(raw),
            Err(e) => {
                return Err(WizardError::Prompt(format!(
                    "existing config at {} could not be parsed: {e}\nFix the config or use --config to specify a different path.",
                    config_path.display()
                )));
            }
        }
    } else {
        None
    };

    let adding_server = existing
        .as_ref()
        .is_some_and(|e| e.servers.as_ref().is_some_and(|s| !s.is_empty()));

    if adding_server {
        if let Some(ref existing) = existing {
            let server_names: Vec<String> = existing
                .servers
                .as_ref()
                .map(|s| s.keys().cloned().collect())
                .unwrap_or_default();
            println!(
                "Found existing config at {} with server(s): {}",
                config_path.display(),
                server_names.join(", ")
            );
        }
        let add_another = inquire::Confirm::new("Add another server?")
            .with_default(true)
            .prompt()
            .map_err(from_inquire)?;
        if !add_another {
            println!(
                "No changes made. Run `smpr rate --help` or edit config at {}",
                config_path.display()
            );
            return Ok(());
        }
    }

    // Step 1: Server connection
    let server_info = server::prompt_server(verbose)?;

    // Step 2: Authentication
    let api_key = auth::prompt_auth(&server_info.url, &server_info.server_type, verbose)?;

    // Construct client for API calls in subsequent steps
    let client = crate::server::MediaServerClient::new(
        server_info.url.clone(),
        api_key.clone(),
        server_info.server_type.clone(),
    );

    // Steps 3-5 only run for fresh config (not when adding a server)
    let (genre_config, detection_config, prefs) = if adding_server {
        // Adding a server — skip detection/genre/preference prompts
        (
            library::GenreConfig { genres: vec![] },
            detection::DetectionAdditions {
                extra_r_stems: vec![],
                extra_r_exact: vec![],
                extra_pg13_stems: vec![],
                extra_pg13_exact: vec![],
                extra_false_positives: vec![],
            },
            preferences::Preferences { overwrite: true },
        )
    } else {
        // Step 3: Library & genre discovery
        let genre_config = library::prompt_library_and_genres(&client, verbose)?;
        // Step 4: Detection rules
        let detection_config = detection::prompt_detection(verbose)?;
        // Step 5: Preferences
        let prefs = preferences::prompt_preferences()?;
        (genre_config, detection_config, prefs)
    };

    // Step 6: Write output
    output::write_config(
        &config_path,
        &env_path,
        existing.as_ref(),
        &server_info,
        &api_key,
        &genre_config,
        &detection_config,
        &prefs,
        adding_server,
    )?;

    println!("\nConfig written to {}", config_path.display());
    println!("API key saved to {}", env_path.display());
    println!("\nTry it: smpr rate --dry-run");

    Ok(())
}
