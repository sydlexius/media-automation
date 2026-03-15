// Typed API response structs. All use #[serde(rename_all = "PascalCase")].

use serde::Deserialize;
use serde_json::Value;

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

/// User info from GET /Users.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserInfo {
    pub id: String,
    pub name: Option<String>,
}

/// Music library from GET /Library/VirtualFolders.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VirtualFolder {
    pub name: String,
    pub item_id: String,
    pub collection_type: Option<String>,
    #[serde(default)]
    pub locations: Vec<String>,
}

/// Single genre from GET /MusicGenres.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenreItem {
    pub name: String,
}

/// Response from GET /MusicGenres.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenreResponse {
    #[serde(default)]
    pub items: Vec<GenreItem>,
}

/// Read-only view of an audio item — deserialized alongside raw Value.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AudioItemView {
    pub id: String,
    pub path: Option<String>,
    pub official_rating: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
}

/// Paginated response from GET /Users/{uid}/Items.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PrefetchResponse {
    #[serde(default)]
    pub items: Vec<Value>,
    #[serde(default)]
    pub total_record_count: i64,
}
