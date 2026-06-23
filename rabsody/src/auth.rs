//! `rabs login` and `rabs config get|set` - native credential management.
//!
//! `login` authenticates against the ABS server and writes a native TOML config;
//! `config get|set` inspects/edits it. Until a native config exists, reads keep
//! working off the abs-cli fallback (see [`crate::config`]).

use clap::{Subcommand, ValueEnum};

use crate::api;
use crate::config::{self, Credentials, StoredConfig};
use crate::error::{Error, Result};

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Print the resolved config (tokens redacted).
    Get,
    /// Set a value: `server`, `library`, or `token`.
    Set { key: ConfigKey, value: String },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ConfigKey {
    Server,
    Library,
    Token,
}

/// `rabs login` - authenticate and write the native config.
pub fn login(server: Option<String>, username: String, password: Option<String>) -> Result<()> {
    let server = match server {
        Some(server) => server,
        // Fall back to the server already in config so `rabs login --username x`
        // works once a server is known. Preserve the load error so a genuine
        // unreadable/malformed config is distinguishable from "no config yet".
        None => Credentials::load().map(|c| c.config.server).map_err(|e| {
            Error::Config(format!(
                "no --server given and could not load an existing config to infer it: {e}"
            ))
        })?,
    };
    let password = match password {
        Some(password) => password,
        None => rpassword::prompt_password(format!("Password for {username}@{server}: "))
            .map_err(|e| Error::Config(format!("reading password: {e}")))?,
    };

    let creds = api::login(&server, &username, &password)?;
    config::save(&creds.config, &creds.source_path)?;
    println!(
        "Logged in as {username}; wrote {}",
        creds.source_path.display()
    );
    if let Some(library) = &creds.config.default_library {
        println!("default library: {library}");
    }
    Ok(())
}

/// `rabs config get|set`.
pub fn config(cmd: ConfigCmd) -> Result<()> {
    match cmd {
        ConfigCmd::Get => {
            let creds = Credentials::load()?;
            let c = &creds.config;
            println!("source:  {}", creds.source_path.display());
            println!("server:  {}", c.server);
            println!(
                "library: {}",
                c.default_library.as_deref().unwrap_or("(none)")
            );
            println!("token:   {}", redacted(!c.access_token.is_empty()));
            println!("refresh: {}", redacted(c.refresh_token.is_some()));
            Ok(())
        }
        ConfigCmd::Set { key, value } => {
            let path = StoredConfig::native_path()?;
            // Seed from the current effective config (native or abs-cli) so a
            // first `config set` graduates it to native without losing fields.
            let mut config = Credentials::load().map(|c| c.config).unwrap_or_default();
            match key {
                ConfigKey::Server => config.server = value,
                ConfigKey::Library => config.default_library = Some(value),
                ConfigKey::Token => config.access_token = value,
            }
            config::save(&config, &path)?;
            println!("updated {} in {}", key.as_str(), path.display());
            Ok(())
        }
    }
}

fn redacted(present: bool) -> &'static str {
    if present { "(set, redacted)" } else { "(none)" }
}

impl ConfigKey {
    fn as_str(self) -> &'static str {
        match self {
            ConfigKey::Server => "server",
            ConfigKey::Library => "library",
            ConfigKey::Token => "token",
        }
    }
}
