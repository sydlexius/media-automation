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
    assert_eq!(info.local_address.as_deref(), Some("http://172.22.0.2:8096"));
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
