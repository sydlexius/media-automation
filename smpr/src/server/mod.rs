// Server module methods are consumed by future milestones (detection, rating).
#![allow(dead_code, unused_imports)]

pub mod error;
pub mod types;

#[cfg(test)]
mod tests;

pub use error::MediaServerError;
pub use types::SystemInfoPublic;

use crate::config::ServerType;
use serde_json::Value;
use std::cell::OnceCell;
use std::time::Duration;

/// HTTP client for Emby/Jellyfin media server APIs.
pub struct MediaServerClient {
    base_url: String,
    api_key: String,
    server_type: ServerType,
    agent: ureq::Agent,
    user_id: OnceCell<String>,
}

impl MediaServerClient {
    /// Create a new client. `server_type` must be resolved before construction
    /// (via `detect_server_type` or TOML override).
    pub fn new(base_url: String, api_key: String, server_type: ServerType) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_per_call(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            server_type,
            agent,
            user_id: OnceCell::new(),
        }
    }

    /// Returns the base URL (trailing slash stripped).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the (header_name, header_value) pair for authentication.
    pub fn auth_header(&self) -> (&str, &str) {
        match self.server_type {
            ServerType::Emby => ("X-Emby-Token", &self.api_key),
            ServerType::Jellyfin => ("X-MediaBrowser-Token", &self.api_key),
        }
    }

    /// Returns the server type.
    pub fn server_type(&self) -> &ServerType {
        &self.server_type
    }

    /// Authenticated JSON request. Returns `Ok(None)` when response body is empty.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>, MediaServerError> {
        let url = format!("{}{}", self.base_url, path);
        let (auth_name, auth_value) = self.auth_header();

        let response = match method {
            "GET" => self
                .agent
                .get(&url)
                .header(auth_name, auth_value)
                .header("Accept", "application/json")
                .call()?,
            "POST" => {
                let req = self
                    .agent
                    .post(&url)
                    .header(auth_name, auth_value)
                    .header("Accept", "application/json");
                match body {
                    Some(b) => req.send_json(b)?,
                    None => req
                        .header("Content-Type", "application/json")
                        .send_empty()?,
                }
            }
            _ => {
                return Err(MediaServerError::Protocol(format!(
                    "unsupported method: {method}"
                )));
            }
        };

        let status = response.status().as_u16();
        if status >= 400 {
            let body_snippet = response.into_body().read_to_string().unwrap_or_default();
            let snippet = if body_snippet.len() > 1024 {
                format!(
                    "{}...",
                    &body_snippet[..body_snippet.floor_char_boundary(1024)]
                )
            } else {
                body_snippet
            };
            return Err(MediaServerError::Http {
                status,
                body: snippet,
            });
        }

        // Read body — empty body returns None
        let body_str = response.into_body().read_to_string().map_err(|e| {
            MediaServerError::Connection(format!("failed to read response body: {e}"))
        })?;
        if body_str.trim().is_empty() {
            return Ok(None);
        }
        let value: Value = serde_json::from_str(&body_str).map_err(|e| {
            MediaServerError::Parse(format!("non-JSON response on {method} {path}: {e}"))
        })?;
        Ok(Some(value))
    }

    /// Authenticated plain-text request. Returns raw response body.
    pub fn request_text(&self, method: &str, path: &str) -> Result<String, MediaServerError> {
        let url = format!("{}{}", self.base_url, path);
        let (auth_name, auth_value) = self.auth_header();

        let response = match method {
            "GET" => self.agent.get(&url).header(auth_name, auth_value).call()?,
            _ => {
                return Err(MediaServerError::Protocol(format!(
                    "unsupported method for request_text: {method}"
                )));
            }
        };

        let status = response.status().as_u16();
        if status >= 400 {
            let body_snippet = response.into_body().read_to_string().unwrap_or_default();
            let snippet = if body_snippet.len() > 1024 {
                format!(
                    "{}...",
                    &body_snippet[..body_snippet.floor_char_boundary(1024)]
                )
            } else {
                body_snippet
            };
            return Err(MediaServerError::Http {
                status,
                body: snippet,
            });
        }

        response
            .into_body()
            .read_to_string()
            .map_err(|e| MediaServerError::Connection(format!("read error: {e}")))
    }

    /// Fetch and cache the first user's ID (needed for user-scoped endpoints).
    pub fn get_user_id(&self) -> Result<&str, MediaServerError> {
        if let Some(id) = self.user_id.get() {
            return Ok(id);
        }
        let users_val = self
            .request("GET", "/Users", None)?
            .ok_or_else(|| MediaServerError::Protocol("no response from /Users".to_string()))?;
        let users: Vec<types::UserInfo> = serde_json::from_value(users_val)
            .map_err(|e| MediaServerError::Parse(format!("/Users response: {e}")))?;
        let first = users
            .first()
            .ok_or_else(|| MediaServerError::Protocol("no users returned from /Users".to_string()))?;
        if first.id.is_empty() {
            return Err(MediaServerError::Protocol(
                "first user has no Id field".to_string(),
            ));
        }
        let _ = self.user_id.set(first.id.clone());
        Ok(self.user_id.get().unwrap())
    }

    /// GET /Users/{userId}/Items/{id} — full item body for round-trip update.
    pub fn get_item(&self, item_id: &str) -> Result<Value, MediaServerError> {
        let uid = self.get_user_id()?;
        let path = format!("/Users/{uid}/Items/{item_id}");
        self.request("GET", &path, None)?
            .ok_or_else(|| {
                MediaServerError::Protocol(format!("empty response for GET {path}"))
            })
    }

    /// POST /Items/{id} — send full item body with modified fields.
    pub fn update_item(&self, item_id: &str, body: &Value) -> Result<(), MediaServerError> {
        let path = format!("/Items/{item_id}");
        self.request("POST", &path, Some(body))?;
        Ok(())
    }
}

/// Determine server type from a parsed SystemInfoPublic and the Server response header.
/// Returns `None` if no signal is conclusive (caller should error with manual-override guidance).
pub fn detect_from_response(info: &SystemInfoPublic, server_header: &str) -> Option<ServerType> {
    // Tier 1: ProductName (official Jellyfin identification mechanism)
    if let Some(product) = &info.product_name {
        if product == "Jellyfin Server" {
            return Some(ServerType::Jellyfin);
        }
        // ProductName present but not Jellyfin → Emby
        return Some(ServerType::Emby);
    }

    // Tier 2: Structural shape (LocalAddress singular vs LocalAddresses plural)
    // Singular takes precedence if both are present
    if info.local_address.is_some() {
        return Some(ServerType::Jellyfin);
    }
    if info.local_addresses.is_some() {
        return Some(ServerType::Emby);
    }

    // Tier 3: Server response header
    if server_header.contains("Kestrel") {
        return Some(ServerType::Jellyfin);
    }
    if !server_header.is_empty() {
        // Any non-empty, non-Kestrel server header → assume Emby
        return Some(ServerType::Emby);
    }

    None
}

/// Auto-detect server type via GET /System/Info/Public (unauthenticated).
/// Returns `ServerType::Emby` or `ServerType::Jellyfin`.
/// Errors if the endpoint is unreachable or no signal can determine the type.
pub fn detect_server_type(url: &str) -> Result<ServerType, MediaServerError> {
    let clean_url = url.trim_end_matches('/');
    let endpoint = format!("{clean_url}/System/Info/Public");

    let agent = ureq::Agent::config_builder()
        .timeout_per_call(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .new_agent();

    let response = agent
        .get(&endpoint)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| {
            MediaServerError::Connection(format!(
                "cannot reach {endpoint} to auto-detect server type: {e}"
            ))
        })?;

    let status = response.status().as_u16();
    if status >= 400 {
        return Err(MediaServerError::Http {
            status,
            body: format!("auto-detection failed at {endpoint}"),
        });
    }

    // Read Server header before consuming body
    let server_header = response
        .headers()
        .get("Server")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body_str = response.into_body().read_to_string().unwrap_or_default();

    let info: SystemInfoPublic = serde_json::from_str(&body_str).unwrap_or_default();

    detect_from_response(&info, &server_header).ok_or_else(|| {
        MediaServerError::Protocol(format!(
            "cannot determine server type at {clean_url}. \
             Set type = \"emby\" or type = \"jellyfin\" in your TOML config."
        ))
    })
}
