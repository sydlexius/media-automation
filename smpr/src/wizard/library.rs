use super::{WizardError, from_inquire};
use crate::server::MediaServerClient;

/// Discovered library with its locations.
pub struct DiscoveredLibrary {
    pub name: String,
    pub locations: Vec<String>,
}

/// Result of the library & genre selection step.
pub struct GenreConfig {
    pub genres: Vec<String>,
    pub libraries: Vec<DiscoveredLibrary>,
}

/// Recommended G-genres for first-time setup.
/// Only genres that are inherently instrumental / guaranteed clean.
pub const DEFAULT_G_GENRES: &[&str] = &[
    "Ambient",
    "Classical",
    "Instrumental",
    "Meditation",
    "New Age",
    "Orchestral",
    "Piano",
];

pub fn prompt_library_and_genres(
    client: &MediaServerClient,
    verbose: bool,
) -> Result<GenreConfig, WizardError> {
    println!("\n── Library & Genre Discovery ──\n");

    let mut discovered_libraries = Vec::new();

    match client.discover_libraries() {
        Ok(libs) => {
            if libs.is_empty() {
                println!("  No music libraries found on this server.");
            } else {
                let names: Vec<&str> = libs.iter().map(|l| l.name.as_str()).collect();
                println!(
                    "  Found {} music library(s): {}",
                    libs.len(),
                    names.join(", ")
                );
                for lib in &libs {
                    if !lib.locations.is_empty() {
                        println!("    {} locations: {}", lib.name, lib.locations.join(", "));
                    }
                    discovered_libraries.push(DiscoveredLibrary {
                        name: lib.name.clone(),
                        locations: lib.locations.clone(),
                    });
                }
            }
        }
        Err(e) => {
            eprintln!("  Warning: could not discover libraries: {e}");
        }
    }

    println!();
    println!(
        "  Default G-rated genres (instrumental/clean):\n    {}",
        DEFAULT_G_GENRES.join(", ")
    );
    println!();

    let options = vec![
        "Use defaults",
        "Skip genre rating",
        "Scan server genres (select from full list)",
    ];
    let choice = inquire::Select::new("Genre defaults:", options)
        .prompt()
        .map_err(from_inquire)?;

    let genres = match choice {
        "Use defaults" => DEFAULT_G_GENRES.iter().map(|s| s.to_string()).collect(),
        "Skip genre rating" => Vec::new(),
        _ => scan_and_select_genres(client, verbose)?,
    };

    Ok(GenreConfig {
        genres,
        libraries: discovered_libraries,
    })
}

fn scan_and_select_genres(
    client: &MediaServerClient,
    verbose: bool,
) -> Result<Vec<String>, WizardError> {
    if verbose {
        eprintln!("Fetching genres from server...");
    }

    let server_genres = client
        .list_genres()
        .map_err(|e| WizardError::ServerUnreachable(format!("could not fetch genres: {e}")))?;

    if server_genres.is_empty() {
        println!("  No genres found on server. Using defaults.");
        return Ok(DEFAULT_G_GENRES.iter().map(|s| s.to_string()).collect());
    }

    println!("  Found {} genres on server.", server_genres.len());

    let default_lower: Vec<String> = DEFAULT_G_GENRES.iter().map(|s| s.to_lowercase()).collect();
    let defaults: Vec<usize> = server_genres
        .iter()
        .enumerate()
        .filter(|(_, g)| default_lower.contains(&g.to_lowercase()))
        .map(|(i, _)| i)
        .collect();

    let selected =
        inquire::MultiSelect::new("Select genres to auto-rate G:", server_genres.clone())
            .with_default(&defaults)
            .with_page_size(20)
            .with_help_message("↑↓ navigate, Space toggle, Type to filter, Enter confirm")
            .prompt()
            .map_err(from_inquire)?;

    Ok(selected)
}
