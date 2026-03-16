use super::WizardError;
use crate::server::MediaServerClient;

/// Result of the genre selection step.
pub struct GenreConfig {
    pub genres: Vec<String>,
}

pub fn prompt_library_and_genres(
    _client: &MediaServerClient,
    _verbose: bool,
) -> Result<GenreConfig, WizardError> {
    todo!("Task 11")
}
