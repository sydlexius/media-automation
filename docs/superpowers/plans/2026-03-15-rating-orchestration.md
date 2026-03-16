# Rating Orchestration Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the rating orchestration layer (issues #79–#83) — the three subcommand workflows (rate/force/reset), CSV report writer, multi-server loop, and summary output.

**Architecture:** Shared scaffolding pattern. Pure functions handle decision logic (testable without a server). Thin wrappers call the server API and delegate to pure logic. Three workflow functions compose the scaffolding with per-item processors. `main.rs` drives the multi-server loop.

**Tech Stack:** Rust 2024 edition, ureq (HTTP), serde_json (JSON), csv (CSV output), env_logger (logging backend), log (logging facade). All dependencies except env_logger already in Cargo.toml.

**Spec:** `docs/superpowers/specs/2026-03-15-rating-orchestration-design.md`

**IMPORTANT agent instructions:**
- Run `cargo fmt` before every commit (pre-commit hooks are bypassed by subagents)
- Run `cargo clippy -- -D warnings` before every commit
- Use `git rm` not `rm` when deleting tracked files
- All tests: `cd /root/Dev/media-automation/tools/smpr && cargo test`
- Integration tests (UAT): `cd /root/Dev/media-automation/tools/smpr && SMPR_UAT_TEST=1 cargo test -- --ignored`

---

## File Structure

| File | Responsibility | Created/Modified |
|------|---------------|-----------------|
| `src/rating.rs` → `src/rating/mod.rs` | Core types, re-exports, workflow functions | Replace stub |
| `src/rating/scope.rs` | Library/location scoping, force_rating lookup | Create |
| `src/rating/action.rs` | Rating decision logic, apply_rating helper | Create |
| `src/rating/tests.rs` | Unit tests for all rating logic | Create |
| `src/report.rs` | CSV report writer | Replace stub |
| `src/main.rs` | Multi-server loop, summary output, CLI wiring | Modify |
| `Cargo.toml` | Add env_logger dependency | Modify |

**Why split `rating.rs` into submodules:** The spec says "starts as a single file" but the combined logic (types + scoping + decisions + 3 workflows + tests) would exceed 800 lines. Splitting by responsibility keeps each file focused and independently reviewable. This is a deliberate deviation from the spec, justified by file size.

**Note on `dead_code` warnings:** Tasks 2-5 introduce types and functions that are not called from `main.rs` until Task 8. Add `#![allow(dead_code)]` at the top of `src/rating/mod.rs` (matching the existing pattern in `config/mod.rs` and `detection.rs`) to prevent clippy failures during intermediate commits. Remove it after Task 8 when the functions are wired into main.

---

## Chunk 1: Core Types and Pure Logic

### Task 1: Add env_logger and initialize logging

**Files:**
- Modify: `smpr/Cargo.toml`
- Modify: `smpr/src/main.rs`

- [ ] **Step 1: Add env_logger dependency**

In `Cargo.toml`, add to `[dependencies]`:
```toml
env_logger = "0.11"
```

- [ ] **Step 2: Initialize logging in main.rs**

Add to `fn main()`. Parse CLI first (to read `--verbose`), then initialize logging:
```rust
use log::LevelFilter;

fn main() {
    let cli = Cli::parse();

    // Determine verbose from any subcommand before initializing logger
    let verbose = match &cli.command {
        Commands::Rate { common, .. }
        | Commands::Force { common, .. }
        | Commands::Reset { common } => common.verbose,
        Commands::Configure { verbose, .. } => *verbose,
    };

    env_logger::Builder::new()
        .filter_level(if verbose {
            LevelFilter::Debug
        } else {
            LevelFilter::Warn
        })
        .format_target(false)
        .format_timestamp(None)
        .init();

    match cli.command {
        // ... existing match arms unchanged
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo build`
Expected: compiles successfully

- [ ] **Step 4: cargo fmt + clippy**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
cd /root/Dev/media-automation/tools/smpr && git add Cargo.toml Cargo.lock src/main.rs && git commit -m "feat: add env_logger and initialize logging backend

Enables log::info!/warn!/debug! output from server module and future
orchestration code. Verbose flag (-v) sets Debug level; default is Warn."
```

---

### Task 2: Define core types (RatingAction, Source, ItemResult, RatingError)

**Files:**
- Create: `smpr/src/rating/mod.rs` (replace `smpr/src/rating.rs`)
- Create: `smpr/src/rating/action.rs`
- Create: `smpr/src/rating/scope.rs`
- Create: `smpr/src/rating/tests.rs`

- [ ] **Step 1: Convert rating.rs to a directory module**

Delete the stub `src/rating.rs` and create the directory structure:

```bash
cd /root/Dev/media-automation/tools/smpr
git rm src/rating.rs
mkdir -p src/rating
```

- [ ] **Step 2: Write src/rating/mod.rs with core types**

```rust
// Rating types and functions are consumed by main.rs workflows.
// Remove this allow after wiring workflows in Task 8.
#![allow(dead_code)]

pub mod action;
pub mod scope;

#[cfg(test)]
mod tests;

use crate::server::MediaServerError;

/// Outcome of processing a single track.
#[derive(Debug, Clone, PartialEq)]
pub enum RatingAction {
    /// Rating was applied to the server.
    Set,
    /// Rating was removed (set to empty string).
    Cleared,
    /// Track was skipped (already rated + skip-existing, or no action needed).
    Skipped,
    /// Rating already matches the desired value.
    AlreadyCorrect,
    /// Dry-run: would have set a rating.
    DryRun,
    /// Dry-run: would have cleared a rating.
    DryRunClear,
    /// Server update failed (non-auth error).
    Error(String),
}

impl RatingAction {
    /// CSV-friendly string representation.
    pub fn as_csv_str(&self) -> &str {
        match self {
            Self::Set => "set",
            Self::Cleared => "cleared",
            Self::Skipped => "skipped",
            Self::AlreadyCorrect => "already_correct",
            Self::DryRun => "dry_run",
            Self::DryRunClear => "dry_run_clear",
            Self::Error(_) => "error",
        }
    }
}

/// Why a track was rated.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Rating determined by lyrics classification.
    Lyrics,
    /// Rating determined by genre allow-list (G).
    Genre,
    /// Force subcommand or config force_rating.
    Force,
    /// Reset subcommand.
    Reset,
}

impl Source {
    /// CSV-friendly string representation.
    pub fn as_csv_str(&self) -> &str {
        match self {
            Self::Lyrics => "lyrics",
            Self::Genre => "genre",
            Self::Force => "force",
            Self::Reset => "reset",
        }
    }
}

/// Result of processing a single audio track.
#[derive(Debug)]
pub struct ItemResult {
    pub item_id: String,
    pub path: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub tier: Option<String>,
    pub matched_words: Vec<String>,
    pub previous_rating: Option<String>,
    pub action: RatingAction,
    pub source: Source,
    pub server_name: String,
}

/// Resolved library/location scope for a workflow.
#[derive(Debug)]
pub struct LibraryScope {
    /// ParentId for prefetch query (None = all items).
    pub parent_id: Option<String>,
    /// Server-side path prefix for post-prefetch location filtering.
    pub location_path: Option<String>,
    /// Resolved library name (for force_rating lookup).
    pub library_name: Option<String>,
}

/// Errors that abort a workflow.
#[derive(Debug)]
pub enum RatingError {
    /// Server API error (non-auth).
    Server(MediaServerError),
    /// Auth error (401/403) — abort immediately.
    Auth(u16),
    /// Requested library not found.
    LibraryNotFound {
        name: String,
        available: Vec<String>,
    },
    /// Requested location not found.
    LocationNotFound {
        name: String,
        available: Vec<String>,
    },
    /// No music libraries found on server.
    NoMusicLibraries,
    /// Library matched but has no ItemId.
    MissingLibraryId(String),
}

impl std::fmt::Display for RatingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Server(e) => write!(f, "{e}"),
            Self::Auth(status) => write!(f, "auth error (HTTP {status})"),
            Self::LibraryNotFound { name, available } => {
                write!(
                    f,
                    "library '{}' not found. Available: {}",
                    name,
                    available.join(", ")
                )
            }
            Self::LocationNotFound { name, available } => {
                write!(
                    f,
                    "location '{}' not found. Available: {}",
                    name,
                    available.join(", ")
                )
            }
            Self::NoMusicLibraries => write!(f, "no music libraries found on server"),
            Self::MissingLibraryId(name) => {
                write!(f, "library '{}' has no ItemId", name)
            }
        }
    }
}

impl std::error::Error for RatingError {}

impl From<MediaServerError> for RatingError {
    fn from(e: MediaServerError) -> Self {
        match &e {
            MediaServerError::Http { status, .. } if *status == 401 || *status == 403 => {
                Self::Auth(*status)
            }
            _ => Self::Server(e),
        }
    }
}
```

- [ ] **Step 3: Create empty submodule files**

`src/rating/action.rs`:
```rust
// Rating decision logic and apply_rating helper.
```

`src/rating/scope.rs`:
```rust
// Library/location scoping and force_rating lookup.
```

`src/rating/tests.rs`:
```rust
use super::*;
```

- [ ] **Step 4: Verify it compiles**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo build`

- [ ] **Step 5: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "feat: define core rating types (RatingAction, Source, ItemResult, RatingError)

Enum-based action/source types replace Python's stringly-typed fields.
RatingError auto-converts from MediaServerError, detecting auth errors.
Split rating module into submodules for scope, action, and tests."
```

---

### Task 3: TDD library/location scoping

**Files:**
- Modify: `smpr/src/rating/scope.rs`
- Modify: `smpr/src/rating/tests.rs`
- Modify: `smpr/src/rating/mod.rs` (re-export)

- [ ] **Step 1: Write failing tests for resolve_from_libraries**

In `src/rating/tests.rs`:

```rust
use super::*;
use crate::server::types::VirtualFolder;

fn music_lib(name: &str, item_id: &str, locations: Vec<&str>) -> VirtualFolder {
    VirtualFolder {
        name: name.to_string(),
        item_id: item_id.to_string(),
        collection_type: Some("music".to_string()),
        locations: locations.into_iter().map(String::from).collect(),
    }
}

#[test]
fn scope_no_flags_returns_none() {
    let libs = vec![music_lib("Music", "lib1", vec!["/music"])];
    let scope = scope::resolve_from_libraries(&libs, None, None).unwrap();
    assert!(scope.parent_id.is_none());
    assert!(scope.location_path.is_none());
}

#[test]
fn scope_library_by_name() {
    let libs = vec![
        music_lib("Music", "lib1", vec!["/music"]),
        music_lib("Audiobooks", "lib2", vec!["/audiobooks"]),
    ];
    let scope = scope::resolve_from_libraries(&libs, Some("Music"), None).unwrap();
    assert_eq!(scope.parent_id.as_deref(), Some("lib1"));
    assert!(scope.location_path.is_none());
    assert_eq!(scope.library_name.as_deref(), Some("Music"));
}

#[test]
fn scope_library_case_insensitive() {
    let libs = vec![music_lib("Music", "lib1", vec!["/music"])];
    let scope = scope::resolve_from_libraries(&libs, Some("music"), None).unwrap();
    assert_eq!(scope.parent_id.as_deref(), Some("lib1"));
}

#[test]
fn scope_library_not_found() {
    let libs = vec![music_lib("Music", "lib1", vec!["/music"])];
    let result = scope::resolve_from_libraries(&libs, Some("Nonexistent"), None);
    assert!(matches!(result, Err(RatingError::LibraryNotFound { .. })));
}

#[test]
fn scope_location_without_library() {
    let libs = vec![music_lib(
        "Music",
        "lib1",
        vec!["/music/rock", "/music/classical"],
    )];
    let scope = scope::resolve_from_libraries(&libs, None, Some("classical")).unwrap();
    assert_eq!(scope.parent_id.as_deref(), Some("lib1"));
    assert_eq!(scope.location_path.as_deref(), Some("/music/classical"));
}

#[test]
fn scope_location_with_library() {
    let libs = vec![music_lib(
        "Music",
        "lib1",
        vec!["/music/rock", "/music/classical"],
    )];
    let scope =
        scope::resolve_from_libraries(&libs, Some("Music"), Some("classical")).unwrap();
    assert_eq!(scope.parent_id.as_deref(), Some("lib1"));
    assert_eq!(scope.location_path.as_deref(), Some("/music/classical"));
}

#[test]
fn scope_location_not_found() {
    let libs = vec![music_lib("Music", "lib1", vec!["/music/rock"])];
    let result = scope::resolve_from_libraries(&libs, Some("Music"), Some("jazz"));
    assert!(matches!(result, Err(RatingError::LocationNotFound { .. })));
}

#[test]
fn scope_no_music_libraries() {
    let libs: Vec<VirtualFolder> = vec![];
    let result = scope::resolve_from_libraries(&libs, Some("Music"), None);
    assert!(matches!(result, Err(RatingError::NoMusicLibraries)));
}

#[test]
fn scope_location_windows_path() {
    let libs = vec![music_lib(
        "Music",
        "lib1",
        vec!["D:\\Music\\classical"],
    )];
    let scope = scope::resolve_from_libraries(&libs, None, Some("classical")).unwrap();
    assert_eq!(scope.location_path.as_deref(), Some("D:\\Music\\classical"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo test rating::tests -- --test-threads=1`
Expected: compilation errors (function doesn't exist yet)

- [ ] **Step 3: Implement resolve_from_libraries in scope.rs**

```rust
use crate::rating::{LibraryScope, RatingError};
use crate::server::types::VirtualFolder;

/// Extract the leaf directory name from a path (handles both / and \ separators).
fn location_leaf(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(pos) => &trimmed[pos + 1..],
        None => trimmed,
    }
}

/// Pure library/location scoping logic. Testable without a server.
///
/// Returns a `LibraryScope` with:
/// - `parent_id`: the ItemId of the matched library (for prefetch ParentId filter)
/// - `location_path`: the full server-side path for post-prefetch filtering
/// - `library_name`: the resolved library name (for force_rating lookup)
pub fn resolve_from_libraries(
    libraries: &[VirtualFolder],
    library_name: Option<&str>,
    location_name: Option<&str>,
) -> Result<LibraryScope, RatingError> {
    if library_name.is_none() && location_name.is_none() {
        return Ok(LibraryScope {
            parent_id: None,
            location_path: None,
            library_name: None,
        });
    }

    if libraries.is_empty() {
        return Err(RatingError::NoMusicLibraries);
    }

    let (lib, matched_location_path) = if let Some(lib_name) = library_name {
        // Find library by name (case-insensitive)
        let lib = libraries
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(lib_name))
            .ok_or_else(|| RatingError::LibraryNotFound {
                name: lib_name.to_string(),
                available: libraries.iter().map(|l| l.name.clone()).collect(),
            })?;

        // If location also specified, find it within this library
        let loc_path = if let Some(loc_name) = location_name {
            let path = lib
                .locations
                .iter()
                .find(|p| location_leaf(p).eq_ignore_ascii_case(loc_name))
                .ok_or_else(|| RatingError::LocationNotFound {
                    name: loc_name.to_string(),
                    available: lib.locations.iter().map(|p| location_leaf(p).to_string()).collect(),
                })?;
            Some(path.clone())
        } else {
            None
        };

        (lib, loc_path)
    } else {
        // --location without --library: search all libraries
        let loc_name = location_name.unwrap();
        let mut found_lib = None;
        let mut found_path = None;
        for lib in libraries {
            for path in &lib.locations {
                if location_leaf(path).eq_ignore_ascii_case(loc_name) {
                    found_lib = Some(lib);
                    found_path = Some(path.clone());
                    break;
                }
            }
            if found_lib.is_some() {
                break;
            }
        }
        match found_lib {
            Some(lib) => (lib, found_path),
            None => {
                let all_locs: Vec<String> = libraries
                    .iter()
                    .flat_map(|l| l.locations.iter().map(|p| location_leaf(p).to_string()))
                    .collect();
                return Err(RatingError::LocationNotFound {
                    name: loc_name.to_string(),
                    available: all_locs,
                });
            }
        }
    };

    if lib.item_id.is_empty() {
        return Err(RatingError::MissingLibraryId(lib.name.clone()));
    }

    log::info!(
        "scoping to library '{}' (ID: {}){}",
        lib.name,
        lib.item_id,
        matched_location_path
            .as_ref()
            .map(|p| format!(", location '{}'", location_leaf(p)))
            .unwrap_or_default()
    );

    Ok(LibraryScope {
        parent_id: Some(lib.item_id.clone()),
        location_path: matched_location_path,
        library_name: Some(lib.name.clone()),
    })
}

/// Post-prefetch filter: keep only items whose path starts with the location path.
pub fn filter_by_location(
    items: Vec<(crate::server::types::AudioItemView, serde_json::Value)>,
    location_path: &str,
) -> Vec<(crate::server::types::AudioItemView, serde_json::Value)> {
    let prefix = normalize_path(location_path.trim_end_matches(['/', '\\']));
    let prefix_with_sep = format!("{prefix}/");
    let before = items.len();
    let filtered: Vec<_> = items
        .into_iter()
        .filter(|(view, _)| {
            view.path
                .as_deref()
                .map(|p| normalize_path(p).starts_with(&prefix_with_sep))
                .unwrap_or(false)
        })
        .collect();
    log::info!(
        "location filter: {} / {} items under {}",
        filtered.len(),
        before,
        location_path,
    );
    filtered
}

/// Normalize path separators to forward slash and lowercase for comparison.
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// Look up force_rating from server config for the given library/location scope.
/// Returns the force_rating string if found, None otherwise.
///
/// Precedence: location force_rating > library force_rating > None.
pub fn lookup_force_rating(
    server_config: &crate::config::ServerConfig,
    library_name: Option<&str>,
    location_name: Option<&str>,
) -> Option<&str> {
    let lib_name = library_name?;
    let lib_config = server_config
        .libraries
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(lib_name))
        .map(|(_, cfg)| cfg)?;

    // Check location-level first
    if let Some(loc_name) = location_name {
        if let Some(loc_config) = lib_config
            .locations
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(loc_name))
            .map(|(_, cfg)| cfg)
        {
            if let Some(ref rating) = loc_config.force_rating {
                return Some(rating.as_str());
            }
        }
    }

    // Fall back to library-level
    lib_config.force_rating.as_deref()
}
```

- [ ] **Step 4: Add re-exports in mod.rs**

At the end of `src/rating/mod.rs`, before the test module reference, add the public re-exports used by main.rs (the scope and action submodules are accessed via `rating::scope::` and `rating::action::` prefixes, so just ensure they're `pub mod`).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo test rating::tests`
Expected: all scope tests pass

- [ ] **Step 6: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "feat: implement library/location scoping and force_rating lookup

Pure functions for resolve_from_libraries, filter_by_location, and
lookup_force_rating. All testable without a server connection.
Case-insensitive matching, Windows path support, detailed error messages."
```

---

### Task 4: Add tests for filter_by_location and lookup_force_rating

**Files:**
- Modify: `smpr/src/rating/tests.rs`

- [ ] **Step 1: Write failing tests for filter_by_location**

Append to `src/rating/tests.rs`:

```rust
use crate::server::types::AudioItemView;
use serde_json::json;

fn audio_item(id: &str, path: &str) -> (AudioItemView, serde_json::Value) {
    let val = json!({
        "Id": id,
        "Path": path,
        "Genres": []
    });
    let view = AudioItemView {
        id: id.to_string(),
        path: Some(path.to_string()),
        official_rating: None,
        album_artist: None,
        album: None,
        genres: vec![],
    };
    (view, val)
}

#[test]
fn filter_location_keeps_matching_paths() {
    let items = vec![
        audio_item("1", "/music/classical/bach.flac"),
        audio_item("2", "/music/rock/acdc.flac"),
        audio_item("3", "/music/classical/mozart.flac"),
    ];
    let filtered = scope::filter_by_location(items, "/music/classical");
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].0.id, "1");
    assert_eq!(filtered[1].0.id, "3");
}

#[test]
fn filter_location_empty_when_no_match() {
    let items = vec![audio_item("1", "/music/rock/acdc.flac")];
    let filtered = scope::filter_by_location(items, "/music/classical");
    assert!(filtered.is_empty());
}

#[test]
fn filter_location_trailing_slash() {
    let items = vec![audio_item("1", "/music/classical/bach.flac")];
    let filtered = scope::filter_by_location(items, "/music/classical/");
    assert_eq!(filtered.len(), 1);
}

#[test]
fn filter_location_case_insensitive() {
    let items = vec![audio_item("1", "/Music/Classical/bach.flac")];
    let filtered = scope::filter_by_location(items, "/music/classical");
    assert_eq!(filtered.len(), 1);
}

#[test]
fn filter_location_windows_backslash() {
    let items = vec![audio_item("1", "D:\\Music\\classical\\bach.flac")];
    let filtered = scope::filter_by_location(items, "D:\\Music\\classical");
    assert_eq!(filtered.len(), 1);
}
```

- [ ] **Step 2: Write failing tests for lookup_force_rating**

Append to `src/rating/tests.rs`:

```rust
use crate::config::{LibraryConfig, LocationConfig, ServerConfig};
use std::collections::BTreeMap;

fn server_with_force_ratings() -> ServerConfig {
    let mut locations = BTreeMap::new();
    locations.insert(
        "classical".to_string(),
        LocationConfig {
            force_rating: Some("G".to_string()),
        },
    );
    let mut libraries = BTreeMap::new();
    libraries.insert(
        "Music".to_string(),
        LibraryConfig {
            force_rating: Some("PG-13".to_string()),
            locations,
        },
    );
    ServerConfig {
        name: "test".to_string(),
        url: "http://localhost".to_string(),
        api_key: "key".to_string(),
        server_type: None,
        libraries,
    }
}

#[test]
fn force_rating_location_overrides_library() {
    let cfg = server_with_force_ratings();
    let rating = scope::lookup_force_rating(&cfg, Some("Music"), Some("classical"));
    assert_eq!(rating, Some("G"));
}

#[test]
fn force_rating_library_fallback() {
    let cfg = server_with_force_ratings();
    let rating = scope::lookup_force_rating(&cfg, Some("Music"), Some("rock"));
    assert_eq!(rating, Some("PG-13"));
}

#[test]
fn force_rating_library_only() {
    let cfg = server_with_force_ratings();
    let rating = scope::lookup_force_rating(&cfg, Some("Music"), None);
    assert_eq!(rating, Some("PG-13"));
}

#[test]
fn force_rating_no_library_match() {
    let cfg = server_with_force_ratings();
    let rating = scope::lookup_force_rating(&cfg, Some("Audiobooks"), None);
    assert_eq!(rating, None);
}

#[test]
fn force_rating_no_library_name() {
    let cfg = server_with_force_ratings();
    let rating = scope::lookup_force_rating(&cfg, None, None);
    assert_eq!(rating, None);
}

#[test]
fn force_rating_case_insensitive() {
    let cfg = server_with_force_ratings();
    let rating = scope::lookup_force_rating(&cfg, Some("music"), Some("Classical"));
    assert_eq!(rating, Some("G"));
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo test rating::tests`
Expected: all tests pass (implementation already in scope.rs from Task 3)

- [ ] **Step 4: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "test: add unit tests for filter_by_location and lookup_force_rating

Covers path matching, case insensitivity, trailing slashes, and
force_rating precedence (location > library > none)."
```

---

### Task 5: TDD rating decision logic

**Files:**
- Modify: `smpr/src/rating/action.rs`
- Modify: `smpr/src/rating/tests.rs`

- [ ] **Step 1: Write failing tests for decide_rating_action**

Append to `src/rating/tests.rs`:

```rust
use super::action;

#[test]
fn decide_already_correct() {
    let result = action::decide_rating_action("R", Some("R"), true, false);
    assert_eq!(result, RatingAction::AlreadyCorrect);
}

#[test]
fn decide_dry_run() {
    let result = action::decide_rating_action("R", Some("G"), true, true);
    assert_eq!(result, RatingAction::DryRun);
}

#[test]
fn decide_skip_existing() {
    let result = action::decide_rating_action("R", Some("G"), false, false);
    assert_eq!(result, RatingAction::Skipped);
}

#[test]
fn decide_set_no_previous() {
    let result = action::decide_rating_action("R", None, true, false);
    assert_eq!(result, RatingAction::Set);
}

#[test]
fn decide_set_different_rating() {
    let result = action::decide_rating_action("R", Some("G"), true, false);
    assert_eq!(result, RatingAction::Set);
}

#[test]
fn decide_set_overwrite_true_no_skip() {
    // overwrite=true, previous exists but different
    let result = action::decide_rating_action("PG-13", Some("R"), true, false);
    assert_eq!(result, RatingAction::Set);
}

#[test]
fn decide_clear_overwrite_has_rating() {
    let result = action::decide_clear_action(Some("R"), true, false);
    assert_eq!(result, RatingAction::Cleared);
}

#[test]
fn decide_clear_no_rating() {
    let result = action::decide_clear_action(None, true, false);
    assert_eq!(result, RatingAction::Skipped);
}

#[test]
fn decide_clear_skip_existing() {
    let result = action::decide_clear_action(Some("R"), false, false);
    assert_eq!(result, RatingAction::Skipped);
}

#[test]
fn decide_clear_dry_run() {
    let result = action::decide_clear_action(Some("R"), true, true);
    assert_eq!(result, RatingAction::DryRunClear);
}

#[test]
fn decide_clear_empty_string_rating() {
    // Empty string previous_rating = no rating
    let result = action::decide_clear_action(Some(""), true, false);
    assert_eq!(result, RatingAction::Skipped);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo test rating::tests`
Expected: compilation errors

- [ ] **Step 3: Implement decision functions in action.rs**

```rust
use crate::rating::RatingAction;
use crate::server::MediaServerError;

/// Decide what action to take for setting a rating.
///
/// Pure logic — no server calls. Returns the action to take.
/// When `Set` is returned, the caller must perform the server round-trip.
pub fn decide_rating_action(
    tier: &str,
    current_rating: Option<&str>,
    overwrite: bool,
    dry_run: bool,
) -> RatingAction {
    // Already at the desired rating?
    if current_rating.is_some_and(|r| r == tier) {
        return RatingAction::AlreadyCorrect;
    }
    // Skip if has existing rating and overwrite is false
    if !overwrite && current_rating.is_some_and(|r| !r.is_empty()) {
        return RatingAction::Skipped;
    }
    if dry_run {
        return RatingAction::DryRun;
    }
    RatingAction::Set
}

/// Decide what action to take for clearing a rating.
///
/// Used when lyrics are clean but a track has an existing rating (overwrite mode).
pub fn decide_clear_action(
    current_rating: Option<&str>,
    overwrite: bool,
    dry_run: bool,
) -> RatingAction {
    // No rating to clear
    if current_rating.is_none() || current_rating.is_some_and(|r| r.is_empty()) {
        return RatingAction::Skipped;
    }
    // Skip-existing mode: don't touch rated tracks
    if !overwrite {
        return RatingAction::Skipped;
    }
    if dry_run {
        return RatingAction::DryRunClear;
    }
    RatingAction::Cleared
}

/// GET-then-POST round-trip to set OfficialRating on an item.
/// Returns the final `RatingAction` (Set, Cleared, or Error).
pub fn apply_rating(
    client: &crate::server::MediaServerClient,
    item_id: &str,
    rating: &str,
    label: &str,
) -> RatingAction {
    match apply_rating_inner(client, item_id, rating) {
        Ok(()) => {
            if rating.is_empty() {
                log::info!("cleared rating from {}", label);
                RatingAction::Cleared
            } else {
                log::info!("set {} on {}", rating, label);
                RatingAction::Set
            }
        }
        Err(e) => {
            log::error!("failed to update {}: {}", label, e);
            RatingAction::Error(e.to_string())
        }
    }
}

fn apply_rating_inner(
    client: &crate::server::MediaServerClient,
    item_id: &str,
    rating: &str,
) -> Result<(), MediaServerError> {
    let mut item = client.get_item(item_id)?;
    item["OfficialRating"] = serde_json::Value::String(rating.to_string());
    client.update_item(item_id, &item)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo test rating::tests`
Expected: all tests pass

- [ ] **Step 5: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "feat: implement rating decision logic and apply_rating helper

decide_rating_action and decide_clear_action are pure functions covering
overwrite/skip-existing/dry-run/already-correct logic. apply_rating does
the GET-then-POST round-trip to set OfficialRating on the server."
```

---

## Chunk 2: Workflows

### Task 6: Implement rate workflow (#79)

**Files:**
- Modify: `smpr/src/rating/mod.rs`

- [ ] **Step 1: Implement rate_workflow in mod.rs**

Add to `src/rating/mod.rs`:

```rust
use crate::config::{Config, ServerConfig, ServerType};
use crate::detection::DetectionEngine;
use crate::server::types::AudioItemView;
use crate::server::{self, MediaServerClient, MediaServerError};
use serde_json::Value;

/// Run the `rate` workflow for a single server.
///
/// Fetches lyrics, classifies content, and sets ratings.
/// Returns results for all items processed.
pub fn rate_workflow(
    client: &MediaServerClient,
    config: &Config,
    server_config: &ServerConfig,
    engine: &DetectionEngine,
) -> Result<Vec<ItemResult>, RatingError> {
    let scope = resolve_library_scope(client, config)?;
    let include_media_sources = client.server_type() == &ServerType::Emby;
    let items = client.prefetch_audio_items(include_media_sources, scope.parent_id.as_deref())?;
    let items = if let Some(ref loc_path) = scope.location_path {
        scope::filter_by_location(items, loc_path)
    } else {
        items
    };

    log::info!("processing {} items for rate workflow", items.len());

    // Check for config-level force_rating (unless --ignore-forced)
    let force_rating = if config.ignore_forced {
        None
    } else {
        scope::lookup_force_rating(
            server_config,
            scope.library_name.as_deref(),
            config.location_name.as_deref(),
        )
    };

    let mut results = Vec::new();
    for (view, raw) in &items {
        let result = rate_item(
            client,
            config,
            engine,
            view,
            raw,
            force_rating,
            &server_config.name,
        )?;
        results.push(result);
    }
    Ok(results)
}

fn rate_item(
    client: &MediaServerClient,
    config: &Config,
    engine: &DetectionEngine,
    view: &AudioItemView,
    raw: &Value,
    force_rating: Option<&str>,
    server_name: &str,
) -> Result<ItemResult, RatingError> {
    let label = view.path.as_deref().unwrap_or(&view.id);
    let prev = view.official_rating.as_deref();

    // Config force_rating takes priority (unless --ignore-forced)
    if let Some(forced) = force_rating {
        let act = action::decide_rating_action(forced, prev, config.overwrite, config.dry_run);
        let act = if matches!(act, RatingAction::Set) {
            action::apply_rating(client, &view.id, forced, label)
        } else {
            act
        };
        return Ok(ItemResult {
            item_id: view.id.clone(),
            path: view.path.clone(),
            artist: view.album_artist.clone(),
            album: view.album.clone(),
            tier: Some(forced.to_string()),
            matched_words: vec![],
            previous_rating: prev.map(String::from),
            action: act,
            source: Source::Force,
            server_name: server_name.to_string(),
        });
    }

    // Fetch lyrics
    let lyrics_text = match client.fetch_lyrics(view, raw) {
        Ok(text) => text,
        Err(MediaServerError::Http { status, .. }) if status == 401 || status == 403 => {
            return Err(RatingError::Auth(status));
        }
        Err(e) => {
            log::warn!("failed to fetch lyrics for {}: {}", label, e);
            None
        }
    };

    if let Some(text) = lyrics_text {
        let (tier, matched) = engine.classify_lyrics(&text);

        if let Some(tier) = tier {
            // Explicit content found
            let act =
                action::decide_rating_action(tier, prev, config.overwrite, config.dry_run);
            let act = if matches!(act, RatingAction::Set) {
                action::apply_rating(client, &view.id, tier, label)
            } else {
                act
            };
            return Ok(ItemResult {
                item_id: view.id.clone(),
                path: view.path.clone(),
                artist: view.album_artist.clone(),
                album: view.album.clone(),
                tier: Some(tier.to_string()),
                matched_words: matched,
                previous_rating: prev.map(String::from),
                action: act,
                source: Source::Lyrics,
                server_name: server_name.to_string(),
            });
        }

        // Clean lyrics — clear existing rating if overwrite enabled
        let act = action::decide_clear_action(prev, config.overwrite, config.dry_run);
        let act = if matches!(act, RatingAction::Cleared) {
            action::apply_rating(client, &view.id, "", label)
        } else {
            act
        };
        return Ok(ItemResult {
            item_id: view.id.clone(),
            path: view.path.clone(),
            artist: view.album_artist.clone(),
            album: view.album.clone(),
            tier: None,
            matched_words: vec![],
            previous_rating: prev.map(String::from),
            action: act,
            source: Source::Lyrics,
            server_name: server_name.to_string(),
        });
    }

    // No lyrics — try genre fallback
    if let Some(matched_genre) = engine.match_g_genre(&view.genres) {
        let act = action::decide_rating_action("G", prev, config.overwrite, config.dry_run);
        let act = if matches!(act, RatingAction::Set) {
            action::apply_rating(client, &view.id, "G", label)
        } else {
            act
        };
        return Ok(ItemResult {
            item_id: view.id.clone(),
            path: view.path.clone(),
            artist: view.album_artist.clone(),
            album: view.album.clone(),
            tier: Some("G".to_string()),
            matched_words: vec![matched_genre.to_string()],
            previous_rating: prev.map(String::from),
            action: act,
            source: Source::Genre,
            server_name: server_name.to_string(),
        });
    }

    // No lyrics, no genre match — skip
    Ok(ItemResult {
        item_id: view.id.clone(),
        path: view.path.clone(),
        artist: view.album_artist.clone(),
        album: view.album.clone(),
        tier: None,
        matched_words: vec![],
        previous_rating: prev.map(String::from),
        action: RatingAction::Skipped,
        source: Source::Lyrics,
        server_name: server_name.to_string(),
    })
}

/// Resolve library/location scope via the server API.
fn resolve_library_scope(
    client: &MediaServerClient,
    config: &Config,
) -> Result<LibraryScope, RatingError> {
    if config.library_name.is_none() && config.location_name.is_none() {
        return Ok(LibraryScope {
            parent_id: None,
            location_path: None,
            library_name: None,
        });
    }
    let libraries = client.discover_libraries()?;
    scope::resolve_from_libraries(
        &libraries,
        config.library_name.as_deref(),
        config.location_name.as_deref(),
    )
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo build`

- [ ] **Step 3: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "feat: implement rate workflow (#79)

Single-pass: for each item, check config force_rating → fetch lyrics →
classify → decide rating action → apply. Genre fallback for tracks
without lyrics. Auth errors abort; other per-item errors continue."
```

---

### Task 7: Implement force and reset workflows (#80, #81)

**Files:**
- Modify: `smpr/src/rating/mod.rs`
- Modify: `smpr/src/rating/tests.rs`

- [ ] **Step 1: Write tests for force/reset decision logic**

Append to `src/rating/tests.rs`:

```rust
#[test]
fn force_skip_existing_with_rating() {
    // overwrite=false, has existing rating → skip
    let result = action::decide_rating_action("G", Some("R"), false, false);
    assert_eq!(result, RatingAction::Skipped);
}

#[test]
fn force_set_no_existing() {
    let result = action::decide_rating_action("G", None, true, false);
    assert_eq!(result, RatingAction::Set);
}

#[test]
fn reset_no_rating_to_clear() {
    let result = action::decide_clear_action(None, true, false);
    assert_eq!(result, RatingAction::Skipped);
}

#[test]
fn reset_clears_existing() {
    let result = action::decide_clear_action(Some("R"), true, false);
    assert_eq!(result, RatingAction::Cleared);
}

#[test]
fn reset_dry_run() {
    let result = action::decide_clear_action(Some("PG-13"), true, true);
    assert_eq!(result, RatingAction::DryRunClear);
}
```

- [ ] **Step 2: Implement force_workflow and reset_workflow in mod.rs**

Add to `src/rating/mod.rs`:

```rust
/// Run the `force` workflow for a single server.
///
/// Sets a fixed rating on all tracks in scope. No lyrics evaluation.
pub fn force_workflow(
    client: &MediaServerClient,
    config: &Config,
    server_config: &ServerConfig,
    target_rating: &str,
) -> Result<Vec<ItemResult>, RatingError> {
    let scope = resolve_library_scope(client, config)?;
    let items = client.prefetch_audio_items(false, scope.parent_id.as_deref())?;
    let items = if let Some(ref loc_path) = scope.location_path {
        scope::filter_by_location(items, loc_path)
    } else {
        items
    };

    log::info!(
        "force-rating {} items as '{}'",
        items.len(),
        target_rating
    );

    let mut results = Vec::new();
    for (view, _) in &items {
        let label = view.path.as_deref().unwrap_or(&view.id);
        let prev = view.official_rating.as_deref();
        let act =
            action::decide_rating_action(target_rating, prev, config.overwrite, config.dry_run);
        let act = if matches!(act, RatingAction::Set) {
            action::apply_rating(client, &view.id, target_rating, label)
        } else {
            act
        };
        results.push(ItemResult {
            item_id: view.id.clone(),
            path: view.path.clone(),
            artist: view.album_artist.clone(),
            album: view.album.clone(),
            tier: Some(target_rating.to_string()),
            matched_words: vec![],
            previous_rating: prev.map(String::from),
            action: act,
            source: Source::Force,
            server_name: server_config.name.clone(),
        });
    }
    Ok(results)
}

/// Run the `reset` workflow for a single server.
///
/// Removes OfficialRating from all tracks in scope.
pub fn reset_workflow(
    client: &MediaServerClient,
    config: &Config,
    server_config: &ServerConfig,
) -> Result<Vec<ItemResult>, RatingError> {
    let scope = resolve_library_scope(client, config)?;
    let items = client.prefetch_audio_items(false, scope.parent_id.as_deref())?;
    let items = if let Some(ref loc_path) = scope.location_path {
        scope::filter_by_location(items, loc_path)
    } else {
        items
    };

    log::info!("resetting ratings on {} items", items.len());

    let mut results = Vec::new();
    for (view, _) in &items {
        let label = view.path.as_deref().unwrap_or(&view.id);
        let prev = view.official_rating.as_deref();
        let act = action::decide_clear_action(prev, true, config.dry_run);
        let act = if matches!(act, RatingAction::Cleared) {
            let applied = action::apply_rating(client, &view.id, "", label);
            // apply_rating returns Set on success; map to Cleared
            if matches!(applied, RatingAction::Set) {
                RatingAction::Cleared
            } else {
                applied
            }
        } else {
            act
        };
        results.push(ItemResult {
            item_id: view.id.clone(),
            path: view.path.clone(),
            artist: view.album_artist.clone(),
            album: view.album.clone(),
            tier: None,
            matched_words: vec![],
            previous_rating: prev.map(String::from),
            action: act,
            source: Source::Reset,
            server_name: server_config.name.clone(),
        });
    }
    Ok(results)
}
```

- [ ] **Step 3: Run tests + verify compile**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo test rating::tests && cargo build
```

- [ ] **Step 4: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "feat: implement force and reset workflows (#80, #81)

force_workflow sets a fixed rating on all items in scope.
reset_workflow removes OfficialRating from all items in scope.
Both respect dry-run and skip-existing flags."
```

---

### Task 8: Wire all workflows in main.rs (single server first)

**Files:**
- Modify: `smpr/src/main.rs`

- [ ] **Step 1: Replace "not yet implemented" with workflow calls**

Update the match arms in `main.rs`:

```rust
use crate::detection::DetectionEngine;
use crate::server::{self, MediaServerClient};

// In the Rate arm:
Commands::Rate {
    common,
    overwrite,
    ignore_forced,
} => {
    let cfg = load_config(&common, overwrite.resolve(), ignore_forced);
    if cfg.verbose {
        eprintln!("Config loaded: {} server(s)", cfg.servers.len());
    }
    let engine = DetectionEngine::new(&cfg.detection);
    let server_config = &cfg.servers[0]; // Single server for now
    let server_type = server_config
        .server_type
        .clone()
        .unwrap_or_else(|| {
            server::detect_server_type(&server_config.url).unwrap_or_else(|e| {
                eprintln!("Error: failed to detect server type for '{}': {e}", server_config.name);
                process::exit(1);
            })
        });
    let client = MediaServerClient::new(
        server_config.url.clone(),
        server_config.api_key.clone(),
        server_type,
    );
    match rating::rate_workflow(&client, &cfg, server_config, &engine) {
        Ok(results) => {
            eprintln!("Processed {} items", results.len());
        }
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}

// In the Force arm:
Commands::Force {
    rating: target_rating,
    common,
    overwrite,
} => {
    let cfg = load_config(&common, overwrite.resolve(), false);
    let server_config = &cfg.servers[0];
    let server_type = server_config
        .server_type
        .clone()
        .unwrap_or_else(|| {
            server::detect_server_type(&server_config.url).unwrap_or_else(|e| {
                eprintln!("Error: failed to detect server type for '{}': {e}", server_config.name);
                process::exit(1);
            })
        });
    let client = MediaServerClient::new(
        server_config.url.clone(),
        server_config.api_key.clone(),
        server_type,
    );
    match rating::force_workflow(&client, &cfg, server_config, &target_rating) {
        Ok(results) => {
            eprintln!("Force-rated {} items", results.len());
        }
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}

// In the Reset arm:
Commands::Reset { common } => {
    let cfg = load_config(&common, None, false);
    let server_config = &cfg.servers[0];
    let server_type = server_config
        .server_type
        .clone()
        .unwrap_or_else(|| {
            server::detect_server_type(&server_config.url).unwrap_or_else(|e| {
                eprintln!("Error: failed to detect server type for '{}': {e}", server_config.name);
                process::exit(1);
            })
        });
    let client = MediaServerClient::new(
        server_config.url.clone(),
        server_config.api_key.clone(),
        server_type,
    );
    match rating::reset_workflow(&client, &cfg, server_config) {
        Ok(results) => {
            eprintln!("Reset {} items", results.len());
        }
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}
```

- [ ] **Step 2: Verify compile + tests**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo build && cargo test
```

- [ ] **Step 3: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "feat: wire rate/force/reset workflows in main.rs

Single-server execution for now. Auto-detects server type if not in
config. Multi-server loop added in a later task."
```

---

## Chunk 3: Report, Multi-Server, Summary

### Task 9: Implement CSV report writer (#82)

**Files:**
- Modify: `smpr/src/report.rs`
- Modify: `smpr/src/rating/tests.rs`

- [ ] **Step 1: Write failing test for write_report**

Append to `src/rating/tests.rs`:

```rust
#[test]
fn report_csv_output() {
    let results = vec![
        ItemResult {
            item_id: "id1".into(),
            path: Some("/music/artist/album/track.flac".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            tier: Some("R".into()),
            matched_words: vec!["word1".into(), "word2".into()],
            previous_rating: Some("G".into()),
            action: RatingAction::Set,
            source: Source::Lyrics,
            server_name: "home-emby".into(),
        },
        ItemResult {
            item_id: "id2".into(),
            path: Some("/music/artist2/album2/clean.flac".into()),
            artist: None,
            album: None,
            tier: None,
            matched_words: vec![],
            previous_rating: None,
            action: RatingAction::Skipped,
            source: Source::Lyrics,
            server_name: "home-emby".into(),
        },
    ];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report.csv");
    crate::report::write_report(&results, &path);
    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines[0], "artist,album,track,tier,matched_words,previous_rating,action,source,server");
    assert!(lines[1].contains("Artist"));
    assert!(lines[1].contains("Album"));
    assert!(lines[1].contains("track.flac"));
    assert!(lines[1].contains("R"));
    assert!(lines[1].contains("word1; word2"));
    assert!(lines[1].contains("set"));
    assert!(lines[1].contains("lyrics"));
    assert!(lines[1].contains("home-emby"));
    assert!(lines[2].contains("clean.flac"));
    assert!(lines[2].contains("skipped"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo test report_csv_output`
Expected: compilation error

- [ ] **Step 3: Implement write_report in report.rs**

Replace the stub `src/report.rs` with:

```rust
use crate::rating::ItemResult;
use std::path::Path;

/// Write detection results to a CSV file.
///
/// Creates parent directories if needed. Errors are logged, not fatal.
pub fn write_report(results: &[ItemResult], path: &Path) {
    if let Err(e) = write_report_inner(results, path) {
        log::error!("cannot write report to {}: {}", path.display(), e);
    } else {
        log::info!("report written to {}", path.display());
    }
}

fn write_report_inner(results: &[ItemResult], path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.write_record([
        "artist",
        "album",
        "track",
        "tier",
        "matched_words",
        "previous_rating",
        "action",
        "source",
        "server",
    ])?;
    for r in results {
        let track = r
            .path
            .as_deref()
            .and_then(|p| p.rsplit(['/', '\\']).next())
            .unwrap_or("");
        wtr.write_record([
            r.artist.as_deref().unwrap_or(""),
            r.album.as_deref().unwrap_or(""),
            track,
            r.tier.as_deref().unwrap_or(""),
            &r.matched_words.join("; "),
            r.previous_rating.as_deref().unwrap_or(""),
            r.action.as_csv_str(),
            r.source.as_csv_str(),
            &r.server_name,
        ])?;
    }
    wtr.flush()?;
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /root/Dev/media-automation/tools/smpr && cargo test report_csv_output`
Expected: PASS

- [ ] **Step 5: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "feat: implement CSV report writer (#82)

Writes one row per track processed with columns: artist, album, track,
tier, matched_words, previous_rating, action, source, server.
Creates parent directories. Errors are logged, not fatal."
```

---

### Task 10: Implement multi-server loop and summary output (#83)

**Files:**
- Modify: `smpr/src/main.rs`
- Modify: `smpr/src/rating/mod.rs` (add print_summary)
- Modify: `smpr/src/rating/tests.rs` (summary tests)

- [ ] **Step 1: Write tests for print_summary counting**

Append to `src/rating/tests.rs`:

```rust
#[test]
fn summary_counts_actions() {
    let results = vec![
        ItemResult {
            item_id: "1".into(), path: None, artist: None, album: None,
            tier: Some("R".into()), matched_words: vec!["fuck".into()],
            previous_rating: None,
            action: RatingAction::Set, source: Source::Lyrics,
            server_name: "s".into(),
        },
        ItemResult {
            item_id: "2".into(), path: None, artist: None, album: None,
            tier: Some("PG-13".into()), matched_words: vec!["bitch".into()],
            previous_rating: None,
            action: RatingAction::DryRun, source: Source::Lyrics,
            server_name: "s".into(),
        },
        ItemResult {
            item_id: "3".into(), path: None, artist: None, album: None,
            tier: Some("G".into()), matched_words: vec!["Classical".into()],
            previous_rating: None,
            action: RatingAction::Set, source: Source::Genre,
            server_name: "s".into(),
        },
        ItemResult {
            item_id: "4".into(), path: None, artist: None, album: None,
            tier: None, matched_words: vec![],
            previous_rating: Some("R".into()),
            action: RatingAction::Cleared, source: Source::Lyrics,
            server_name: "s".into(),
        },
        ItemResult {
            item_id: "5".into(), path: None, artist: None, album: None,
            tier: Some("R".into()), matched_words: vec![],
            previous_rating: Some("R".into()),
            action: RatingAction::AlreadyCorrect, source: Source::Lyrics,
            server_name: "s".into(),
        },
        ItemResult {
            item_id: "6".into(), path: None, artist: None, album: None,
            tier: None, matched_words: vec![],
            previous_rating: None,
            action: RatingAction::Skipped, source: Source::Lyrics,
            server_name: "s".into(),
        },
        ItemResult {
            item_id: "7".into(), path: None, artist: None, album: None,
            tier: Some("G".into()), matched_words: vec!["Ambient".into()],
            previous_rating: Some("G".into()),
            action: RatingAction::AlreadyCorrect, source: Source::Genre,
            server_name: "s".into(),
        },
    ];
    let counts = SummaryCounts::from_results(&results);
    assert_eq!(counts.lyrics_evaluated, 5);  // source=Lyrics
    assert_eq!(counts.r_rated, 2);           // tier=R
    assert_eq!(counts.pg13, 1);              // tier=PG-13
    assert_eq!(counts.clean, 2);             // source=Lyrics, tier=None
    assert_eq!(counts.ratings_set, 1);       // action=Set, source=Lyrics
    assert_eq!(counts.already_correct, 1);   // action=AlreadyCorrect, source=Lyrics
    assert_eq!(counts.cleared, 1);
    assert_eq!(counts.g_genre_set, 1);       // action=Set, source=Genre
    assert_eq!(counts.g_genre_already, 1);   // action=AlreadyCorrect, source=Genre
    assert_eq!(counts.dry_run, 1);
    assert_eq!(counts.skipped, 1);
    assert_eq!(counts.errors, 0);
}
```

- [ ] **Step 2: Implement SummaryCounts and print_summary**

Add to `src/rating/mod.rs`:

```rust
/// Counts for summary output.
#[derive(Debug, Default)]
pub struct SummaryCounts {
    pub lyrics_evaluated: usize,
    pub r_rated: usize,
    pub pg13: usize,
    pub clean: usize,
    pub ratings_set: usize,
    pub already_correct: usize,
    pub cleared: usize,
    pub g_genre_set: usize,
    pub g_genre_already: usize,
    pub g_genre_dry: usize,
    pub dry_run: usize,
    pub skipped: usize,
    pub errors: usize,
}

impl SummaryCounts {
    pub fn from_results(results: &[ItemResult]) -> Self {
        let mut c = Self::default();
        for r in results {
            // Lyrics evaluated = source is Lyrics
            if r.source == Source::Lyrics {
                c.lyrics_evaluated += 1;
            }
            // Tier counts
            match r.tier.as_deref() {
                Some("R") => c.r_rated += 1,
                Some("PG-13") => c.pg13 += 1,
                _ => {}
            }
            // Clean = has lyrics but no explicit content
            if r.source == Source::Lyrics && r.tier.is_none() {
                c.clean += 1;
            }
            // Action counts by source
            match (&r.action, &r.source) {
                (RatingAction::Set, Source::Genre) => c.g_genre_set += 1,
                (RatingAction::Set, _) => c.ratings_set += 1,
                (RatingAction::AlreadyCorrect, Source::Genre) => c.g_genre_already += 1,
                (RatingAction::AlreadyCorrect, _) => c.already_correct += 1,
                (RatingAction::Cleared, _) => c.cleared += 1,
                (RatingAction::DryRun, Source::Genre) => c.g_genre_dry += 1,
                (RatingAction::DryRun, _) => c.dry_run += 1,
                (RatingAction::DryRunClear, _) => c.dry_run += 1,
                (RatingAction::Skipped, _) => c.skipped += 1,
                (RatingAction::Error(_), _) => c.errors += 1,
            }
        }
        c
    }
}

/// Print summary counts to stdout.
pub fn print_summary(results: &[ItemResult], label: &str) {
    let c = SummaryCounts::from_results(results);
    if !label.is_empty() {
        println!("\n=== {} ===", label);
    }
    println!();
    println!("=== Rating Summary ===");
    if c.lyrics_evaluated > 0 {
        println!("  Lyrics evaluated:    {}", c.lyrics_evaluated);
        println!("    R-rated:           {}", c.r_rated);
        println!("    PG-13:             {}", c.pg13);
        println!("    Clean:             {}", c.clean);
    }
    println!("  Ratings set:         {}", c.ratings_set);
    println!("  Already correct:     {}", c.already_correct);
    println!("  Ratings cleared:     {}", c.cleared);
    if c.g_genre_set > 0 || c.g_genre_already > 0 || c.g_genre_dry > 0 {
        println!("  G (genre-matched):   {}", c.g_genre_set);
        println!("  Already G (genre):   {}", c.g_genre_already);
        if c.g_genre_dry > 0 {
            println!("  Dry-run G (genre):   {}", c.g_genre_dry);
        }
    }
    if c.skipped > 0 {
        println!("  Skipped:             {}", c.skipped);
    }
    if c.dry_run > 0 {
        println!("  Dry-run would act:   {}", c.dry_run);
    }
    println!("  Errors:              {}", c.errors);
}
```

- [ ] **Step 3: Run test to verify summary counting**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo test summary_counts_actions
```
Expected: PASS

- [ ] **Step 4: Implement multi-server loop in main.rs**

Replace the single-server match arms with the full multi-server loop. Extract a shared function:

```rust
fn run_workflows(cfg: &config::Config, command: &Commands) {
    let multi = cfg.servers.len() > 1;
    let mut all_results: Vec<rating::ItemResult> = Vec::new();
    let mut had_failure = false;

    for server_config in &cfg.servers {
        let label = if multi {
            format!(
                "{} ({})",
                server_config.name,
                server_config
                    .server_type
                    .as_ref()
                    .map(|t| format!("{t:?}"))
                    .unwrap_or_else(|| "auto".into())
            )
        } else {
            String::new()
        };
        if multi {
            eprintln!("--- Processing {} ---", label);
        }

        let server_type = match server_config.server_type.clone() {
            Some(t) => t,
            None => match server::detect_server_type(&server_config.url) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!(
                        "Error: failed to detect server type for '{}': {e}",
                        server_config.name
                    );
                    had_failure = true;
                    continue;
                }
            },
        };

        let client = server::MediaServerClient::new(
            server_config.url.clone(),
            server_config.api_key.clone(),
            server_type,
        );

        let results = match command {
            Commands::Rate { ignore_forced, .. } => {
                let engine = DetectionEngine::new(&cfg.detection);
                rating::rate_workflow(&client, cfg, server_config, &engine)
            }
            Commands::Force {
                rating: target_rating,
                ..
            } => rating::force_workflow(&client, cfg, server_config, target_rating),
            Commands::Reset { .. } => rating::reset_workflow(&client, cfg, server_config),
            Commands::Configure { .. } => unreachable!(),
        };

        match results {
            Ok(results) => {
                if multi {
                    rating::print_summary(&results, &label);
                }
                all_results.extend(results);
            }
            Err(e) => {
                eprintln!(
                    "Error: {} failed: {e}",
                    if label.is_empty() { "Server" } else { &label }
                );
                had_failure = true;
            }
        }
    }

    // Write report
    if let Some(ref report_path) = cfg.report_path {
        crate::report::write_report(&all_results, report_path);
    }

    // Print summary (single server, or overall for multi)
    if !multi {
        rating::print_summary(&all_results, "");
    }

    if had_failure {
        process::exit(1);
    }
}
```

Update the main match arms to call `run_workflows`:

```rust
Commands::Rate { common, overwrite, ignore_forced } => {
    let cfg = load_config(&common, overwrite.resolve(), ignore_forced);
    run_workflows(&cfg, &cli.command);
}
Commands::Force { ref common, ref overwrite, .. } => {
    let cfg = load_config(common, overwrite.resolve(), false);
    run_workflows(&cfg, &cli.command);
}
Commands::Reset { ref common } => {
    let cfg = load_config(common, None, false);
    run_workflows(&cfg, &cli.command);
}
Commands::Configure { .. } => {
    eprintln!("configure: not yet implemented");
    process::exit(1);
}
```

- [ ] **Step 5: Run all tests + verify compile**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo build && cargo test
```

- [ ] **Step 6: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "feat: implement multi-server loop and summary output (#83)

Iterates configured servers, auto-detects type, runs the chosen workflow,
collects results. Per-server error isolation (continues to next server).
Summary printed per-server when multi-server, once when single server.
Combined CSV report written after all servers."
```

---

### Task 11: Remove dead_code allows from used items

**Files:**
- Modify: `smpr/src/detection.rs`
- Modify: `smpr/src/server/mod.rs`

- [ ] **Step 1: Remove `#[allow(dead_code)]` from items now used by rating module**

In `detection.rs`: remove `#[allow(dead_code)]` from `DetectionEngine`, `new()`, `classify_lyrics()`, `match_g_genre()`, and the helper functions called by these.

In `server/mod.rs`: remove `#![allow(dead_code, unused_imports)]` module-level allow (or narrow it to only truly unused items).

- [ ] **Step 2: Verify compile + tests**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo build && cargo test && cargo clippy -- -D warnings
```

- [ ] **Step 3: cargo fmt + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt
git add -A && git commit -m "chore: remove dead_code allows from items now used by rating module"
```

---

### Task 12: Integration tests (UAT dry-run)

**Files:**
- Modify: `smpr/src/rating/tests.rs`

- [ ] **Step 1: Add integration tests gated behind SMPR_UAT_TEST**

Append to `src/rating/tests.rs`:

```rust
/// Integration tests — UAT servers only. Gated behind SMPR_UAT_TEST=1.
/// All tests are READ-ONLY (dry-run). No mutations to UAT data.
#[cfg(test)]
mod integration {
    use super::*;
    use crate::config::{Config, DetectionConfig, ServerConfig, ServerType};
    use crate::detection::DetectionEngine;
    use crate::server::{self, MediaServerClient};
    use std::collections::BTreeMap;

    fn uat_jellyfin_client() -> MediaServerClient {
        dotenvy::from_filename("../../.env").ok();
        let api_key = std::env::var("UAT_JELLYFIN_API_KEY")
            .expect("UAT_JELLYFIN_API_KEY must be set for integration tests");
        MediaServerClient::new(
            "http://localhost:8097".into(),
            api_key,
            ServerType::Jellyfin,
        )
    }

    fn dry_run_config() -> Config {
        Config {
            servers: vec![],
            detection: DetectionConfig {
                r_stems: vec!["fuck".into(), "shit".into()],
                r_exact: vec!["blowjob".into()],
                pg13_stems: vec!["bitch".into()],
                pg13_exact: vec!["hoe".into()],
                false_positives: vec!["cocktail".into()],
                g_genres: vec!["Classical".into()],
            },
            overwrite: true,
            dry_run: true,
            report_path: None,
            library_name: Some("Music".into()),
            location_name: None,
            verbose: false,
            ignore_forced: false,
        }
    }

    fn test_server_config() -> ServerConfig {
        ServerConfig {
            name: "uat-jellyfin".into(),
            url: "http://localhost:8097".into(),
            api_key: String::new(),
            server_type: Some(ServerType::Jellyfin),
            libraries: BTreeMap::new(),
        }
    }

    #[test]
    #[ignore] // Run with: SMPR_UAT_TEST=1 cargo test -- --ignored
    fn uat_rate_dry_run() {
        let client = uat_jellyfin_client();
        let cfg = dry_run_config();
        let srv = test_server_config();
        let engine = DetectionEngine::new(&cfg.detection);
        let results = rate_workflow(&client, &cfg, &srv, &engine).unwrap();
        assert!(!results.is_empty(), "expected at least one item");
        // All should be dry-run (no Set or Cleared)
        for r in &results {
            assert!(
                !matches!(r.action, RatingAction::Set | RatingAction::Cleared),
                "dry-run should not mutate: {:?} on {}",
                r.action,
                r.item_id
            );
        }
    }

    #[test]
    #[ignore]
    fn uat_force_dry_run() {
        let client = uat_jellyfin_client();
        let cfg = dry_run_config();
        let srv = test_server_config();
        let results = force_workflow(&client, &cfg, &srv, "G").unwrap();
        assert!(!results.is_empty());
        for r in &results {
            assert!(matches!(
                r.action,
                RatingAction::DryRun | RatingAction::AlreadyCorrect
            ));
            assert_eq!(r.source, Source::Force);
        }
    }

    #[test]
    #[ignore]
    fn uat_reset_dry_run() {
        let client = uat_jellyfin_client();
        let cfg = dry_run_config();
        let srv = test_server_config();
        let results = reset_workflow(&client, &cfg, &srv).unwrap();
        assert!(!results.is_empty());
        for r in &results {
            assert!(matches!(
                r.action,
                RatingAction::DryRunClear | RatingAction::Skipped
            ));
            assert_eq!(r.source, Source::Reset);
        }
    }
}
```

- [ ] **Step 2: Run unit tests (should still pass)**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo test
```

- [ ] **Step 3: Run integration tests against UAT**

```bash
cd /root/Dev/media-automation/tools/smpr && SMPR_UAT_TEST=1 cargo test -- --ignored
```
Expected: all 3 integration tests pass

- [ ] **Step 4: cargo fmt + clippy + commit**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo fmt && cargo clippy -- -D warnings
git add -A && git commit -m "test: add UAT integration tests for rate/force/reset dry-run

Read-only tests against Jellyfin at localhost:8097. Gated behind
SMPR_UAT_TEST=1. Verify dry-run produces no mutations."
```

---

### Task 13: Final verification and cleanup

- [ ] **Step 1: Run full test suite**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo test
```

- [ ] **Step 2: Run clippy with all warnings as errors**

```bash
cd /root/Dev/media-automation/tools/smpr && cargo clippy -- -D warnings
```

- [ ] **Step 3: Manual smoke test against UAT**

```bash
cd /root/Dev/media-automation/tools && \
  cargo run --manifest-path smpr/Cargo.toml -- rate \
    --server-url http://localhost:8097 \
    --api-key "$(grep UAT_JELLYFIN_API_KEY .env | cut -d= -f2)" \
    --library Music --dry-run -v
```

Expected: items processed, summary printed, no errors, no mutations.

- [ ] **Step 4: Smoke test CSV report**

```bash
cd /root/Dev/media-automation/tools && \
  cargo run --manifest-path smpr/Cargo.toml -- rate \
    --server-url http://localhost:8097 \
    --api-key "$(grep UAT_JELLYFIN_API_KEY .env | cut -d= -f2)" \
    --library Music --dry-run --report /tmp/smpr-report.csv && \
  head -5 /tmp/smpr-report.csv
```

Expected: CSV file created with header and data rows.

---

## PR Strategy

The commits above are structured for 4 PRs matching the spec:

| PR | Tasks | Branch Name | Issues |
|----|-------|-------------|--------|
| PR 1 | Tasks 1–8 | `feat/rate-workflow` | #79 |
| PR 2 | Tasks 7 (force/reset parts) | `feat/force-reset-workflows` | #80, #81 |
| PR 3 | Task 9 | `feat/csv-report` | #82 |
| PR 4 | Tasks 10–13 | `feat/multi-server-summary` | #83 |

**Note:** Tasks are numbered for implementation order, not PR grouping. The agent should create PRs after completing the relevant task group, using `gt` for stacking if needed.

**Merge protocol:** Wait for CodeRabbit to finish. Dismiss stale `CHANGES_REQUESTED` reviews + `@coderabbitai approve` before merging. NEVER use `gh pr merge --admin`.

**After squash-merging a base PR:** Cherry-pick unique commits for dependent PRs instead of rebasing.
