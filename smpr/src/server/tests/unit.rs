use super::super::error::MediaServerError;

#[test]
fn error_display_http() {
    let err = MediaServerError::Http {
        status: 404,
        body: "Not Found".to_string(),
    };
    assert_eq!(err.to_string(), "HTTP 404: Not Found");
}

#[test]
fn error_display_connection() {
    let err = MediaServerError::Connection("timeout".to_string());
    assert_eq!(err.to_string(), "connection error: timeout");
}

#[test]
fn error_display_parse() {
    let err = MediaServerError::Parse("invalid json".to_string());
    assert_eq!(err.to_string(), "parse error: invalid json");
}

#[test]
fn error_display_protocol() {
    let err = MediaServerError::Protocol("no users".to_string());
    assert_eq!(err.to_string(), "protocol error: no users");
}

use super::super::MediaServerClient;
use crate::config::ServerType;

#[test]
fn auth_header_emby() {
    let client = MediaServerClient::new(
        "http://localhost:8096".to_string(),
        "test-key".to_string(),
        ServerType::Emby,
    );
    let (name, value) = client.auth_header();
    assert_eq!(name, "X-Emby-Token");
    assert_eq!(value, "test-key");
}

#[test]
fn auth_header_jellyfin() {
    let client = MediaServerClient::new(
        "http://localhost:8097".to_string(),
        "test-key".to_string(),
        ServerType::Jellyfin,
    );
    let (name, value) = client.auth_header();
    assert_eq!(name, "X-MediaBrowser-Token");
    assert_eq!(value, "test-key");
}

#[test]
fn base_url_trailing_slash_stripped() {
    let client = MediaServerClient::new(
        "http://localhost:8096/".to_string(),
        "key".to_string(),
        ServerType::Emby,
    );
    assert_eq!(client.base_url(), "http://localhost:8096");
}

use super::super::types::SystemInfoPublic;

#[test]
fn parse_system_info_jellyfin() {
    let json = r#"{
        "LocalAddress": "http://172.22.0.2:8096",
        "ServerName": "jellyfin-test",
        "Version": "10.11.6",
        "ProductName": "Jellyfin Server",
        "OperatingSystem": "",
        "Id": "4b873737cd2b4629bca0db6243058554",
        "StartupWizardCompleted": true
    }"#;
    let info: SystemInfoPublic = serde_json::from_str(json).unwrap();
    assert_eq!(info.product_name.as_deref(), Some("Jellyfin Server"));
    assert_eq!(
        info.local_address.as_deref(),
        Some("http://172.22.0.2:8096")
    );
    assert!(info.local_addresses.is_none());
    assert_eq!(info.startup_wizard_completed, Some(true));
}

#[test]
fn parse_system_info_emby() {
    let json = r#"{
        "LocalAddresses": [],
        "RemoteAddresses": [],
        "ServerName": "OutaTime",
        "Version": "4.10.0.5",
        "Id": "909da4331156415eb771a38a9658aafc"
    }"#;
    let info: SystemInfoPublic = serde_json::from_str(json).unwrap();
    assert!(info.product_name.is_none());
    assert!(info.local_address.is_none());
    assert!(info.local_addresses.is_some());
    assert_eq!(info.server_name.as_deref(), Some("OutaTime"));
}

#[test]
fn parse_system_info_unknown_fields_ignored() {
    let json = r#"{"ServerName": "test", "UnknownField": 42, "AnotherOne": true}"#;
    let info: SystemInfoPublic = serde_json::from_str(json).unwrap();
    assert_eq!(info.server_name.as_deref(), Some("test"));
}

use super::super::detect_from_response;

// Tier 1: ProductName
#[test]
fn detect_jellyfin_by_product_name() {
    let info = SystemInfoPublic {
        product_name: Some("Jellyfin Server".to_string()),
        ..Default::default()
    };
    assert_eq!(detect_from_response(&info, ""), Some(ServerType::Jellyfin));
}

#[test]
fn detect_emby_by_product_name_other() {
    let info = SystemInfoPublic {
        product_name: Some("Emby Server".to_string()),
        ..Default::default()
    };
    assert_eq!(detect_from_response(&info, ""), Some(ServerType::Emby));
}

// Tier 2: Structural shape
#[test]
fn detect_jellyfin_by_local_address_singular() {
    let info = SystemInfoPublic {
        local_address: Some("http://172.22.0.2:8096".to_string()),
        ..Default::default()
    };
    assert_eq!(detect_from_response(&info, ""), Some(ServerType::Jellyfin));
}

#[test]
fn detect_emby_by_local_addresses_plural() {
    let info = SystemInfoPublic {
        local_addresses: Some(vec![]),
        ..Default::default()
    };
    assert_eq!(detect_from_response(&info, ""), Some(ServerType::Emby));
}

#[test]
fn detect_jellyfin_singular_takes_precedence_over_plural() {
    let info = SystemInfoPublic {
        local_address: Some("http://example.com".to_string()),
        local_addresses: Some(vec![]),
        ..Default::default()
    };
    assert_eq!(detect_from_response(&info, ""), Some(ServerType::Jellyfin));
}

// Tier 3: Server header
#[test]
fn detect_jellyfin_by_kestrel_header() {
    let info = SystemInfoPublic::default();
    assert_eq!(
        detect_from_response(&info, "Kestrel"),
        Some(ServerType::Jellyfin)
    );
}

#[test]
fn detect_emby_by_upnp_header() {
    let info = SystemInfoPublic::default();
    assert_eq!(
        detect_from_response(&info, "UPnP/1.0 DLNADOC/1.50"),
        Some(ServerType::Emby)
    );
}

// Tier 4: No signal
#[test]
fn detect_none_when_no_signals() {
    let info = SystemInfoPublic::default();
    assert_eq!(detect_from_response(&info, ""), None);
}

use super::super::types::{
    AudioItemView, GenreResponse, PrefetchResponse, UserInfo, VirtualFolder,
};

#[test]
fn parse_user_info() {
    let json = r#"{"Id": "af181ce70817479a88f588e7adc321c7", "Name": "Librarian"}"#;
    let user: UserInfo = serde_json::from_str(json).unwrap();
    assert_eq!(user.id, "af181ce70817479a88f588e7adc321c7");
    assert_eq!(user.name.as_deref(), Some("Librarian"));
}

#[test]
fn parse_user_info_extra_fields_ignored() {
    let json = r#"{"Id": "abc123", "Name": "Test", "HasPassword": true, "Policy": {}}"#;
    let user: UserInfo = serde_json::from_str(json).unwrap();
    assert_eq!(user.id, "abc123");
}

#[test]
fn parse_virtual_folder_music() {
    let json = r#"{
        "Name": "Music",
        "Locations": ["/classical/", "/music/"],
        "CollectionType": "music",
        "ItemId": "7990",
        "LibraryOptions": {}
    }"#;
    let lib: VirtualFolder = serde_json::from_str(json).unwrap();
    assert_eq!(lib.name, "Music");
    assert_eq!(lib.item_id, "7990");
    assert_eq!(lib.collection_type.as_deref(), Some("music"));
    assert_eq!(lib.locations.len(), 2);
}

#[test]
fn parse_genre_response() {
    let json = r#"{
        "Items": [
            {"Name": "Rock", "Id": "8088", "Type": "MusicGenre"},
            {"Name": "Hip-Hop", "Id": "8081", "Type": "MusicGenre"}
        ],
        "TotalRecordCount": 2
    }"#;
    let resp: GenreResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.items.len(), 2);
    assert_eq!(resp.items[0].name, "Rock");
}

#[test]
fn parse_audio_item_view_emby() {
    let json = r#"{
        "Id": "9383",
        "Path": "/music/Watashi Wa/Eager Seas/01. Track.mp3",
        "Genres": ["Alternative Rock", "rock"],
        "Album": "Eager Seas",
        "AlbumArtist": "Watashi Wa",
        "Type": "Audio",
        "MediaType": "Audio"
    }"#;
    let item: AudioItemView = serde_json::from_str(json).unwrap();
    assert_eq!(item.id, "9383");
    assert_eq!(item.path.as_deref(), Some("/music/Watashi Wa/Eager Seas/01. Track.mp3"));
    assert!(item.official_rating.is_none());
    assert_eq!(item.genres.len(), 2);
}

#[test]
fn parse_audio_item_view_jellyfin() {
    let json = r#"{
        "Id": "7526015b24dce6f8fc732af486395856",
        "Path": "/music/Watashi Wa/Eager Seas/01. Track.mp3",
        "OfficialRating": "R",
        "Genres": ["Rock"],
        "Album": "Eager Seas",
        "AlbumArtist": "Watashi Wa",
        "HasLyrics": false,
        "ChannelId": null
    }"#;
    let item: AudioItemView = serde_json::from_str(json).unwrap();
    assert_eq!(item.id, "7526015b24dce6f8fc732af486395856");
    assert_eq!(item.official_rating.as_deref(), Some("R"));
}

#[test]
fn parse_prefetch_response() {
    let json = r#"{
        "Items": [
            {"Id": "1", "Name": "Track 1"},
            {"Id": "2", "Name": "Track 2"}
        ],
        "TotalRecordCount": 100
    }"#;
    let resp: PrefetchResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.items.len(), 2);
    assert_eq!(resp.total_record_count, 100);
}

use super::super::extract_audio_items;

#[test]
fn extract_items_from_value_array() {
    let items: Vec<serde_json::Value> = serde_json::from_str(r#"[
        {"Id": "1", "Path": "/music/a.mp3", "Genres": []},
        {"Id": "2", "Path": "/music/b.mp3", "Genres": ["Rock"]}
    ]"#).unwrap();
    let pairs = extract_audio_items(&items);
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].0.id, "1");
    assert_eq!(pairs[1].0.genres, vec!["Rock"]);
}

#[test]
fn extract_items_skips_unparseable() {
    let items = vec![
        serde_json::json!({"Id": "1", "Path": "/a.mp3", "Genres": []}),
        serde_json::json!({"NotAnItem": true}),
        serde_json::json!({"Id": "3", "Path": "/c.mp3", "Genres": []}),
    ];
    let pairs = extract_audio_items(&items);
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].0.id, "1");
    assert_eq!(pairs[1].0.id, "3");
}
