//! Credential + config resolution for RABSody.
//!
//! Two on-disk sources, native preferred:
//!   1. Native RABSody config at `<config-dir>/rabsody/config.toml` (written by
//!      `rabs login` / `rabs config set`).
//!   2. abs-cli's `~/.abs-cli/config.json` - the fallback "until native auth
//!      lands" (reads keep working off it).
//!
//! One [`StoredConfig`] serves both via serde; load/save/persist pick TOML vs
//! JSON by file extension, so the native config is TOML while the abs-cli file
//! stays JSON, and refreshed tokens persist back to whichever supplied them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The on-disk credential shape shared by the native and abs-cli config files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredConfig {
    pub server: String,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_library: Option<String>,
}

impl StoredConfig {
    /// `<config-dir>/rabsody/config.toml` (e.g. `~/.config/rabsody/config.toml`).
    pub fn native_path() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .ok_or_else(|| Error::Config("could not resolve a config directory".to_string()))?;
        Ok(dir.join("rabsody").join("config.toml"))
    }

    /// abs-cli's `~/.abs-cli/config.json`.
    pub fn abscli_path() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| Error::Config("could not resolve the home directory".to_string()))?;
        Ok(home.join(".abs-cli").join("config.json"))
    }

    fn load_path(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("reading config at {}: {e}", path.display())))?;
        if is_toml(path) {
            toml::from_str(&raw).map_err(|e| {
                Error::Config(format!("parsing TOML config at {}: {e}", path.display()))
            })
        } else {
            serde_json::from_str(&raw).map_err(|e| {
                Error::Config(format!("parsing JSON config at {}: {e}", path.display()))
            })
        }
    }
}

/// True when `path` is a `.toml` file (else treated as JSON).
fn is_toml(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

/// Resolved credentials plus the file they came from (where refreshes persist).
#[derive(Debug, Clone)]
pub struct Credentials {
    pub config: StoredConfig,
    pub source_path: PathBuf,
}

impl Credentials {
    /// Native config if it exists, else the abs-cli fallback.
    pub fn load() -> Result<Self> {
        let native = StoredConfig::native_path()?;
        if native.exists() {
            let config = StoredConfig::load_path(&native)?;
            return Ok(Self {
                config,
                source_path: native,
            });
        }
        let abscli = StoredConfig::abscli_path()?;
        let config = StoredConfig::load_path(&abscli)?;
        Ok(Self {
            config,
            source_path: abscli,
        })
    }
}

/// Write a [`StoredConfig`] to `path` (creating parents) with `0600` perms,
/// since it holds tokens. TOML for `.toml`, otherwise pretty JSON.
pub fn save(config: &StoredConfig, path: &Path) -> Result<()> {
    let body = if is_toml(path) {
        toml::to_string_pretty(config)
            .map_err(|e| Error::Config(format!("serializing TOML config: {e}")))?
    } else {
        serde_json::to_string_pretty(config)
            .map_err(|e| Error::Config(format!("serializing JSON config: {e}")))?
    };
    write_secret(path, &body)
}

/// Update only the tokens in `path`, preserving any other fields, then re-write
/// with `0600` perms. Used after a transparent refresh so both RABSody and
/// abs-cli see the rotated tokens. For TOML this round-trips the full
/// [`StoredConfig`]; for JSON it merges to keep any extra abs-cli keys.
pub fn persist_tokens(path: &Path, access: &str, refresh: Option<&str>) -> Result<()> {
    if is_toml(path) {
        let mut config = StoredConfig::load_path(path).unwrap_or_default();
        config.access_token = access.to_string();
        if let Some(refresh) = refresh {
            config.refresh_token = Some(refresh.to_string());
        }
        return save(&config, path);
    }
    let mut value: serde_json::Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let obj = value.as_object_mut().ok_or_else(|| {
        Error::Config(format!("config at {} is not a JSON object", path.display()))
    })?;
    obj.insert(
        "accessToken".to_string(),
        serde_json::Value::String(access.to_string()),
    );
    if let Some(refresh) = refresh {
        obj.insert(
            "refreshToken".to_string(),
            serde_json::Value::String(refresh.to_string()),
        );
    }
    let json = serde_json::to_string_pretty(&value)
        .map_err(|e| Error::Config(format!("serializing JSON config: {e}")))?;
    write_secret(path, &json)
}

fn write_secret(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Config(format!("creating config dir {}: {e}", parent.display())))?;
    }
    std::fs::write(path, body)
        .map_err(|e| Error::Config(format!("writing config at {}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best-effort: tokens live here, so restrict to the owner.
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_toml_by_extension() {
        assert!(is_toml(Path::new("/x/config.toml")));
        assert!(is_toml(Path::new("/x/config.TOML")));
        assert!(!is_toml(Path::new("/x/config.json")));
        assert!(!is_toml(Path::new("/x/config")));
    }

    #[test]
    fn toml_round_trips_camelcase_and_omits_none() {
        let cfg = StoredConfig {
            server: "https://abs.example".to_string(),
            access_token: "atk".to_string(),
            refresh_token: None,
            default_library: Some("lib1".to_string()),
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(s.contains("accessToken ="));
        assert!(s.contains("defaultLibrary ="));
        assert!(!s.contains("refreshToken")); // None is skipped
        let back: StoredConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.server, cfg.server);
        assert_eq!(back.default_library.as_deref(), Some("lib1"));
        assert!(back.refresh_token.is_none());
    }

    #[test]
    fn persist_tokens_json_preserves_extra_keys() {
        let dir = std::env::temp_dir().join(format!("rabsody-cfgtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("abscli.json");
        std::fs::write(
            &path,
            r#"{"server":"s","accessToken":"old","extra":"keep","defaultLibrary":"L"}"#,
        )
        .unwrap();

        persist_tokens(&path, "newatk", Some("newrt")).unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["accessToken"], "newatk");
        assert_eq!(v["refreshToken"], "newrt");
        assert_eq!(v["extra"], "keep"); // unrelated abs-cli key preserved
        assert_eq!(v["defaultLibrary"], "L");
        std::fs::remove_dir_all(&dir).ok();
    }
}
