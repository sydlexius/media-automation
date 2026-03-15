mod defaults;
#[cfg(test)]
mod tests;

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Default)]
pub struct RawConfig {
    pub servers: Option<HashMap<String, RawServerConfig>>,
    pub detection: Option<RawDetection>,
    pub general: Option<RawGeneral>,
    pub report: Option<RawReport>,
}

#[derive(Debug, Deserialize)]
pub struct RawServerConfig {
    pub url: Option<String>,
    #[serde(rename = "type")]
    pub server_type: Option<String>,
    pub libraries: Option<HashMap<String, RawLibraryConfig>>,
}

#[derive(Debug, Deserialize)]
pub struct RawLibraryConfig {
    pub force_rating: Option<String>,
    pub locations: Option<HashMap<String, RawLocationConfig>>,
}

#[derive(Debug, Deserialize)]
pub struct RawLocationConfig {
    pub force_rating: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RawDetection {
    pub r: Option<RawWordList>,
    pub pg13: Option<RawWordList>,
    pub ignore: Option<RawIgnore>,
    pub g_genres: Option<RawGenres>,
}

#[derive(Debug, Deserialize)]
pub struct RawWordList {
    pub stems: Option<Vec<String>>,
    pub exact: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RawIgnore {
    pub false_positives: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RawGenres {
    pub genres: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RawGeneral {
    pub overwrite: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RawReport {
    pub output_path: Option<String>,
}

pub fn parse_toml(content: &str) -> Result<RawConfig, toml::de::Error> {
    toml::from_str(content)
}

// ── Resolved config types ──────────────────────────────────────────

#[derive(Debug)]
pub struct Config {
    pub servers: Vec<ServerConfig>,
    pub detection: DetectionConfig,
    pub overwrite: bool,
    pub dry_run: bool,
    pub report_path: Option<PathBuf>,
    pub library_name: Option<String>,
    pub location_name: Option<String>,
    pub verbose: bool,
    pub ignore_forced: bool,
}

#[derive(Debug)]
pub struct ServerConfig {
    pub name: String,
    pub url: String,
    pub api_key: String,
    pub server_type: Option<ServerType>,
    pub libraries: HashMap<String, LibraryConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerType {
    Emby,
    Jellyfin,
}

#[derive(Debug)]
pub struct LibraryConfig {
    pub force_rating: Option<String>,
    pub locations: HashMap<String, LocationConfig>,
}

#[derive(Debug)]
pub struct LocationConfig {
    pub force_rating: Option<String>,
}

#[derive(Debug)]
pub struct DetectionConfig {
    pub r_stems: Vec<String>,
    pub r_exact: Vec<String>,
    pub pg13_stems: Vec<String>,
    pub pg13_exact: Vec<String>,
    pub false_positives: Vec<String>,
    pub g_genres: Vec<String>,
}
