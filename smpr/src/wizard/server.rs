use super::{WizardError, from_inquire};
use crate::config::ServerType;
use crate::server;
use inquire::autocompletion::Replacement;

/// Result of the server connection step.
pub struct ServerInfo {
    pub url: String,
    pub label: String,
    pub server_type: ServerType,
}

/// Common server URLs suggested during autocomplete.
const COMMON_URLS: &[&str] = &[
    "http://localhost:8096",
    "http://localhost:8097",
    "https://localhost:8920",
    "https://localhost:8096",
    "https://localhost:8097",
];

#[derive(Clone, Default)]
struct UrlAutocomplete;

impl inquire::Autocomplete for UrlAutocomplete {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, inquire::CustomUserError> {
        let input_lower = input.to_lowercase();
        let suggestions: Vec<String> = COMMON_URLS
            .iter()
            .filter(|url| url.starts_with(&input_lower))
            .map(|s| s.to_string())
            .collect();
        Ok(suggestions)
    }

    fn get_completion(
        &mut self,
        _input: &str,
        highlighted_suggestion: Option<String>,
    ) -> Result<Replacement, inquire::CustomUserError> {
        Ok(highlighted_suggestion)
    }
}

fn validate_url(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    let lower = trimmed.to_lowercase();
    if let Some(rest) = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    {
        if rest.is_empty() || rest == "/" {
            return Err("URL must include a hostname after the scheme".to_string());
        }
        Ok(trimmed.to_string())
    } else {
        Err("URL must start with http:// or https://".to_string())
    }
}

fn suggest_label(url: &str) -> String {
    let host_port = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("server");
    host_port
        .split(':')
        .next()
        .unwrap_or("server")
        .replace('.', "-")
        .to_lowercase()
}

fn validate_label(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Label cannot be empty".to_string());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Label can only contain letters, numbers, hyphens, and underscores".to_string(),
        );
    }
    Ok(trimmed.to_string())
}

pub fn prompt_server(verbose: bool) -> Result<ServerInfo, WizardError> {
    println!("\n── Server Connection ──\n");

    let url = inquire::Text::new("Server URL:")
        .with_placeholder("http://localhost:8096")
        .with_autocomplete(UrlAutocomplete)
        .with_help_message("Tab to autocomplete, or type your own URL")
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
        .with_validator(|input: &str| match validate_label(input) {
            Ok(_) => Ok(inquire::validator::Validation::Valid),
            Err(e) => Ok(inquire::validator::Validation::Invalid(e.into())),
        })
        .prompt()
        .map_err(from_inquire)?
        .trim()
        .to_string();

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
        assert_eq!(suggest_label("http://host:8096/emby"), "host");
    }
}
