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
