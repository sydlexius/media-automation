use super::*;
use crate::server::types::{AudioItemView, VirtualFolder};
use serde_json::json;

fn music_lib(name: &str, item_id: &str, locations: Vec<&str>) -> VirtualFolder {
    VirtualFolder {
        name: name.to_string(),
        item_id: item_id.to_string(),
        collection_type: Some("music".to_string()),
        locations: locations.into_iter().map(String::from).collect(),
    }
}

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

// ── resolve_from_libraries tests ────────────────────────────────────

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
    let scope = scope::resolve_from_libraries(&libs, Some("Music"), Some("classical")).unwrap();
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
    let libs = vec![music_lib("Music", "lib1", vec!["D:\\Music\\classical"])];
    let scope = scope::resolve_from_libraries(&libs, None, Some("classical")).unwrap();
    assert_eq!(scope.location_path.as_deref(), Some("D:\\Music\\classical"));
}

// ── filter_by_location tests ────────────────────────────────────────

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

// ── lookup_force_rating tests ───────────────────────────────────────

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

// ── decide_rating_action tests ──────────────────────────────────────

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
    let result = action::decide_rating_action("PG-13", Some("R"), true, false);
    assert_eq!(result, RatingAction::Set);
}

// ── decide_clear_action tests ───────────────────────────────────────

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
    let result = action::decide_clear_action(Some(""), true, false);
    assert_eq!(result, RatingAction::Skipped);
}

// Additional force/reset decision tests

#[test]
fn force_skip_existing_with_rating() {
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
    assert_eq!(
        lines[0],
        "artist,album,track,tier,matched_words,previous_rating,action,source,server"
    );
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
