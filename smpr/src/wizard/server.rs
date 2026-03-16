use super::{WizardError, from_inquire};
use crate::config::ServerType;
use crate::server;

/// Result of the server connection step.
pub struct ServerInfo {
    pub url: String,
    pub label: String,
    pub server_type: ServerType,
}

fn validate_url(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(trimmed.to_string())
    } else {
        Err("URL must start with http:// or https://".to_string())
    }
}

fn suggest_label(url: &str) -> String {
    url.trim_start_matches("http://")
        .trim_start_matches("https://")
        .split(':')
        .next()
        .unwrap_or("server")
        .replace('.', "-")
        .to_lowercase()
}

pub fn prompt_server(verbose: bool) -> Result<ServerInfo, WizardError> {
    println!("\n── Server Connection ──\n");

    let url = inquire::Text::new("Server URL:")
        .with_placeholder("http://localhost:8096")
        .with_validator(|input: &str| match validate_url(input) {
            Ok(_) => Ok(inquire::validator::Validation::Valid),
            Err(e) => Ok(inquire::validator::Validation::Invalid(e.into())),
        })
        .prompt()
        .map_err(from_inquire)?;

    let url = url.trim().trim_end_matches('/').to_string();

    if verbose {
        eprintln!("Detecting server type at {url}...");
    }
    let server_type = match server::detect_server_type(&url) {
        Ok(st) => {
            println!("  Detected: {st:?}");
            st
        }
        Err(e) => {
            eprintln!("  Could not auto-detect server type: {e}");
            let options = vec!["Emby", "Jellyfin"];
            let choice = inquire::Select::new("Server type:", options)
                .prompt()
                .map_err(from_inquire)?;
            match choice {
                "Emby" => ServerType::Emby,
                "Jellyfin" => ServerType::Jellyfin,
                _ => unreachable!(),
            }
        }
    };

    let default_label = suggest_label(&url);
    let label = inquire::Text::new("Label for this server:")
        .with_default(&default_label)
        .with_help_message("Used in config file section name and env var prefix")
        .prompt()
        .map_err(from_inquire)?;

    Ok(ServerInfo {
        url,
        label,
        server_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_label_from_url() {
        assert_eq!(suggest_label("http://localhost:8096"), "localhost");
        assert_eq!(suggest_label("http://home-server:8096"), "home-server");
        assert_eq!(
            suggest_label("https://media.example.com:443"),
            "media-example-com"
        );
        assert_eq!(suggest_label("http://192.168.1.126:8096"), "192-168-1-126");
    }
}
