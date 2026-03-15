//! Integration tests — UAT servers only.
//! Gated behind SMPR_UAT_TEST=1 env var.
//! Servers: localhost:8096 (Emby), localhost:8097 (Jellyfin).

use super::super::detect_server_type;
use crate::config::ServerType;

fn uat_enabled() -> bool {
    std::env::var("SMPR_UAT_TEST").map_or(false, |v| v == "1")
}

#[test]
fn detect_emby_uat() {
    if !uat_enabled() {
        eprintln!("skipping: SMPR_UAT_TEST not set");
        return;
    }
    let result = detect_server_type("http://localhost:8096");
    assert!(result.is_ok(), "detection failed: {:?}", result.err());
    assert_eq!(result.unwrap(), ServerType::Emby);
}

#[test]
fn detect_jellyfin_uat() {
    if !uat_enabled() {
        eprintln!("skipping: SMPR_UAT_TEST not set");
        return;
    }
    let result = detect_server_type("http://localhost:8097");
    assert!(result.is_ok(), "detection failed: {:?}", result.err());
    assert_eq!(result.unwrap(), ServerType::Jellyfin);
}

#[test]
fn detect_unreachable_returns_error() {
    if !uat_enabled() {
        eprintln!("skipping: SMPR_UAT_TEST not set");
        return;
    }
    let result = detect_server_type("http://localhost:19999");
    assert!(result.is_err());
}
