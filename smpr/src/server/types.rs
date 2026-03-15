// Typed API response structs. All use #[serde(rename_all = "PascalCase")].

use serde::Deserialize;

/// Response from GET /System/Info/Public (unauthenticated).
/// Both Emby and Jellyfin serve this endpoint but with different fields.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct SystemInfoPublic {
    pub product_name: Option<String>,
    pub server_name: Option<String>,
    pub version: Option<String>,
    pub id: Option<String>,
    pub local_address: Option<String>,
    pub local_addresses: Option<Vec<String>>,
    pub startup_wizard_completed: Option<bool>,
}
