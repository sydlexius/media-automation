#![allow(dead_code)]

use crate::config::RawConfig;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

pub fn label_to_env_var(label: &str) -> String {
    format!("{}_API_KEY", label.to_uppercase().replace('-', "_"))
}

pub fn save_config(config: &RawConfig, path: &Path) -> io::Result<()> {
    let content = toml::to_string_pretty(config).map_err(io::Error::other)?;
    let tmp_path = path.with_extension("toml.tmp");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&tmp_path, &content)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

pub fn load_env_keys(
    env_path: &Path,
    server_labels: &[String],
) -> io::Result<BTreeMap<String, String>> {
    let mut keys = BTreeMap::new();
    if !env_path.is_file() {
        return Ok(keys);
    }
    let content = fs::read_to_string(env_path)?;
    for label in server_labels {
        let env_var = label_to_env_var(label);
        for line in content.lines() {
            let line = line.trim();
            if let Some(value) = line
                .strip_prefix(&env_var)
                .and_then(|rest| rest.strip_prefix('='))
            {
                keys.insert(label.clone(), value.to_string());
                break;
            }
        }
    }
    Ok(keys)
}

pub fn save_env(
    env_keys: &BTreeMap<String, String>,
    known_labels: &[String],
    path: &Path,
) -> io::Result<()> {
    let managed_vars: BTreeMap<String, &str> = env_keys
        .iter()
        .map(|(label, key)| (label_to_env_var(label), key.as_str()))
        .collect();

    let deleted_vars: std::collections::HashSet<String> = known_labels
        .iter()
        .filter(|label| !env_keys.contains_key(*label))
        .map(|label| label_to_env_var(label))
        .collect();

    let existing = if path.is_file() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    let mut output_lines: Vec<String> = Vec::new();
    let mut written_vars: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in existing.lines() {
        let trimmed = line.trim();

        let managed_match = managed_vars.iter().find(|(var, _)| {
            trimmed.starts_with(var.as_str()) && trimmed[var.len()..].starts_with('=')
        });

        let is_deleted = deleted_vars
            .iter()
            .any(|var| trimmed.starts_with(var.as_str()) && trimmed[var.len()..].starts_with('='));

        if let Some((var, value)) = managed_match {
            output_lines.push(format!("{var}={value}"));
            written_vars.insert(var.clone());
        } else if is_deleted {
            // Server was deleted — remove this line
        } else {
            output_lines.push(line.to_string());
        }
    }

    for (var, value) in &managed_vars {
        if !written_vars.contains(var.as_str()) {
            output_lines.push(format!("{var}={value}"));
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = output_lines.join("\n");
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }

    // Atomic write: write to tmp, rename over original
    let tmp_path = path.with_extension("env.tmp");
    fs::write(&tmp_path, &content)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}
