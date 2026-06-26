//! Build script: derive the binary version from the git tag so the release tag
//! is the single source of truth. `Cargo.toml`'s `version` is a frozen `0.0.0`
//! placeholder; the real version is resolved here, in precedence order:
//!
//! 1. `SMPR_VERSION_OVERRIDE` env var (CI passes the stripped tag, so release
//!    builds need no git history / tag fetch).
//! 2. `git describe --tags --match 'smpr-v*' --dirty`, with the `smpr-v` prefix
//!    stripped (e.g. `smpr-v0.4.1` -> `0.4.1`, `0.4.1-3-gabc123-dirty` between).
//! 3. `CARGO_PKG_VERSION` (the `0.0.0` placeholder) when git is unavailable.
//!
//! The result is exposed to the crate as `env!("SMPR_VERSION")`.

use std::path::Path;
use std::process::Command;

fn main() {
    let version = resolve_version();
    println!("cargo:rustc-env=SMPR_VERSION={version}");
    println!("cargo:rerun-if-env-changed=SMPR_VERSION_OVERRIDE");
    // Refresh the embedded version when the git ref state changes. Paths are
    // relative to the crate root (`.git` lives one level up). Only emit watches
    // for paths that exist so an absent `.git` (source tarball) does not force a
    // perpetual rebuild.
    for rel in ["../.git/HEAD", "../.git/packed-refs", "../.git/refs/tags"] {
        if Path::new(rel).exists() {
            println!("cargo:rerun-if-changed={rel}");
        }
    }
}

fn resolve_version() -> String {
    if let Ok(override_version) = std::env::var("SMPR_VERSION_OVERRIDE") {
        let trimmed = override_version.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Some(described) = git_describe() {
        return described;
    }
    std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string())
}

/// `git describe` against the `smpr-v*` tag namespace, with the prefix stripped.
/// Returns `None` on any failure (no git, no matching tag, shallow clone) so the
/// caller falls through to the `CARGO_PKG_VERSION` placeholder rather than
/// failing the build.
fn git_describe() -> Option<String> {
    let output = Command::new("git")
        .args(["describe", "--tags", "--match", "smpr-v*", "--dirty"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let described = String::from_utf8(output.stdout).ok()?;
    let described = described.trim();
    let stripped = described.strip_prefix("smpr-v").unwrap_or(described);
    (!stripped.is_empty()).then(|| stripped.to_string())
}
