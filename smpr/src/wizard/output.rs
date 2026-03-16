use super::WizardError;
use super::detection::DetectionAdditions;
use super::library::GenreConfig;
use super::preferences::Preferences;
use super::server::ServerInfo;
use crate::config::RawConfig;
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub fn write_config(
    _config_path: &Path,
    _env_path: &Path,
    _existing: Option<&RawConfig>,
    _server: &ServerInfo,
    _api_key: &str,
    _genres: &GenreConfig,
    _detection: &DetectionAdditions,
    _prefs: &Preferences,
) -> Result<(), WizardError> {
    todo!("Task 14")
}
