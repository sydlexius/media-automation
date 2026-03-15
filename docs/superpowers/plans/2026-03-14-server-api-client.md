# Server API Client Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the Python `MediaServerClient` to Rust, providing HTTP client, server type detection, item CRUD, lyrics fetch, library discovery, and authentication for Emby/Jellyfin servers.

**Architecture:** A `server/` module directory with `error.rs` (error enum), `types.rs` (typed API response structs), and `mod.rs` (client struct + public methods). Uses `ureq` v3 for blocking HTTP, `serde_json::Value` for round-trip item bodies, typed structs for read-only responses. Interior mutability via `OnceCell` for cached user ID.

**Tech Stack:** Rust 1.94+ (edition 2024), ureq 3 (blocking HTTP with `json` feature), serde/serde_json (serialization), config module's `ServerType` enum (already defined).

**Spec:** `docs/superpowers/specs/2026-03-14-server-api-client-design.md`

**Python reference:** `SetMusicParentalRating/SetMusicParentalRating.py` lines 216–996

---

## File Structure

### New files (PR 1)
- `smpr/src/server/mod.rs` — `MediaServerClient` struct, `new()`, `request()`, `request_text()`, `detect_server_type()`, re-exports
- `smpr/src/server/error.rs` — `MediaServerError` enum with `Display`, `Error`, `From<ureq::Error>`
- `smpr/src/server/types.rs` — `SystemInfoPublic` (PR 1 only; other types added in later PRs)
- `smpr/src/server/tests/mod.rs` — test module routing
- `smpr/src/server/tests/unit.rs` — canned JSON tests (no network)
- `smpr/src/server/tests/integration.rs` — UAT-only tests gated behind `SMPR_UAT_TEST=1`

### New files (PR 3)
- `smpr/src/util.rs` — `strip_lrc_tags()` shared utility

### Modified files
- `smpr/src/main.rs` — `mod server` becomes directory module (PR 1); add `mod util` (PR 3)

### Deleted files
- `smpr/src/server.rs` — replaced by `smpr/src/server/mod.rs`

---

## Chunk 1: PR 1 — HTTP Core + Auto-Detection (#70 + #69)

### Task 1: Module scaffold and error type

**Files:**
- Delete: `smpr/src/server.rs`
- Create: `smpr/src/server/mod.rs`
- Create: `smpr/src/server/error.rs`
- Create: `smpr/src/server/types.rs`
- Create: `smpr/src/server/tests/mod.rs`
- Create: `smpr/src/server/tests/unit.rs`
- Create: `smpr/src/server/tests/integration.rs`
- Modify: `smpr/src/main.rs`

- [ ] **Step 1: Delete the stub `server.rs` and create the module directory**

```bash
rm smpr/src/server.rs
mkdir -p smpr/src/server/tests
```

- [ ] **Step 2: Create `smpr/src/server/error.rs` with `MediaServerError`**

```rust
use std::fmt;

/// Error type for all media server API operations.
#[derive(Debug)]
pub enum MediaServerError {
    /// HTTP error with status code and response body snippet.
    Http { status: u16, body: String },
    /// Server unreachable or request timed out.
    Connection(String),
    /// Response body is not valid JSON.
    Parse(String),
    /// Valid JSON but missing expected fields.
    Protocol(String),
}

impl fmt::Display for MediaServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { status, body } => write!(f, "HTTP {status}: {body}"),
            Self::Connection(msg) => write!(f, "connection error: {msg}"),
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
        }
    }
}

impl std::error::Error for MediaServerError {}

impl From<ureq::Error> for MediaServerError {
    fn from(err: ureq::Error) -> Self {
        match err {
            ureq::Error::StatusCode(code) => Self::Http {
                status: code,
                body: String::new(),
            },
            other => Self::Connection(other.to_string()),
        }
    }
}
```

- [ ] **Step 2b: Add `log` crate dependency**

Run: `cd smpr && cargo add log`

The `log` crate is used for `log::info!`, `log::warn!`, `log::debug!` throughout
the server module. It's a facade — the binary picks a backend (e.g. `env_logger`)
later. For now, log calls are no-ops unless a backend is initialized.

- [ ] **Step 3: Create empty `smpr/src/server/types.rs`**

```rust
// Typed API response structs. All use #[serde(rename_all = "PascalCase")].
```

- [ ] **Step 4: Create `smpr/src/server/mod.rs` with re-exports**

```rust
// Server module methods are consumed by future milestones (detection, rating).
#![allow(dead_code)]

pub mod error;
pub mod types;

#[cfg(test)]
mod tests;

pub use error::MediaServerError;
```

- [ ] **Step 5: Create test module files**

`smpr/src/server/tests/mod.rs`:
```rust
mod unit;

#[cfg(test)]
mod integration;
```

`smpr/src/server/tests/unit.rs`:
```rust
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
```

`smpr/src/server/tests/integration.rs`:
```rust
// Integration tests gated behind SMPR_UAT_TEST=1 env var.
// UAT servers only: localhost:8096 (Emby), localhost:8097 (Jellyfin).
```

- [ ] **Step 6: Update `smpr/src/main.rs` — no changes needed**

The existing `mod server;` line already works — Rust resolves it to `server/mod.rs` when the directory exists and the file is deleted. Verify this compiles.

- [ ] **Step 7: Run tests to verify scaffold works**

Run: `cd smpr && cargo test -- server::tests::unit`
Expected: 4 tests pass (error Display tests)

- [ ] **Step 8: Commit**

```bash
git add -A smpr/src/server/
git commit -m "feat(server): scaffold module directory and MediaServerError (#70)"
```

---

### Task 2: SystemInfoPublic type

**Files:**
- Modify: `smpr/src/server/types.rs`
- Modify: `smpr/src/server/tests/unit.rs`

- [ ] **Step 1: Write failing test for SystemInfoPublic deserialization**

Add to `smpr/src/server/tests/unit.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd smpr && cargo test -- server::tests::unit::parse_system_info`
Expected: FAIL — `SystemInfoPublic` not defined yet

- [ ] **Step 3: Implement SystemInfoPublic**

Write in `smpr/src/server/types.rs`:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd smpr && cargo test -- server::tests::unit::parse_system_info`
Expected: 3 tests pass

- [ ] **Step 5: Commit**

```bash
git add smpr/src/server/types.rs smpr/src/server/tests/unit.rs
git commit -m "feat(server): add SystemInfoPublic response type (#69)"
```

---

### Task 3: MediaServerClient struct and request methods

**Files:**
- Modify: `smpr/src/server/mod.rs`
- Modify: `smpr/src/server/tests/unit.rs`

- [ ] **Step 1: Write failing test for auth header selection**

Add to `smpr/src/server/tests/unit.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd smpr && cargo test -- server::tests::unit::auth_header`
Expected: FAIL — `MediaServerClient` not defined

- [ ] **Step 3: Implement MediaServerClient struct**

Replace contents of `smpr/src/server/mod.rs`:

```rust
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
            let body_snippet = response
                .into_body()
                .read_to_string()
                .unwrap_or_default();
            let snippet = if body_snippet.len() > 1024 {
                format!("{}...", &body_snippet[..1024])
            } else {
                body_snippet
            };
            return Err(MediaServerError::Http {
                status,
                body: snippet,
            });
        }

        // Read body — empty body returns None
        let body_str = response
            .into_body()
            .read_to_string()
            .unwrap_or_default();
        if body_str.trim().is_empty() {
            return Ok(None);
        }
        let value: Value = serde_json::from_str(&body_str).map_err(|e| {
            MediaServerError::Parse(format!(
                "non-JSON response on {method} {path}: {e}"
            ))
        })?;
        Ok(Some(value))
    }

    /// Authenticated plain-text request. Returns raw response body.
    pub fn request_text(
        &self,
        method: &str,
        path: &str,
    ) -> Result<String, MediaServerError> {
        let url = format!("{}{}", self.base_url, path);
        let (auth_name, auth_value) = self.auth_header();

        let response = match method {
            "GET" => self
                .agent
                .get(&url)
                .header(auth_name, auth_value)
                .call()?,
            _ => {
                return Err(MediaServerError::Protocol(format!(
                    "unsupported method for request_text: {method}"
                )));
            }
        };

        let status = response.status().as_u16();
        if status >= 400 {
            let body_snippet = response
                .into_body()
                .read_to_string()
                .unwrap_or_default();
            let snippet = if body_snippet.len() > 1024 {
                format!("{}...", &body_snippet[..1024])
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
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd smpr && cargo test -- server::tests::unit`
Expected: all tests pass (error tests + auth header + base_url tests)

- [ ] **Step 5: Commit**

```bash
git add smpr/src/server/mod.rs smpr/src/server/tests/unit.rs
git commit -m "feat(server): add MediaServerClient with request/request_text (#70)"
```

---

### Task 4: Server type auto-detection

**Files:**
- Modify: `smpr/src/server/mod.rs`
- Modify: `smpr/src/server/tests/unit.rs`

- [ ] **Step 1: Write failing tests for detection logic**

Add to `smpr/src/server/tests/unit.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd smpr && cargo test -- server::tests::unit::detect`
Expected: FAIL — `detect_from_response` not defined

- [ ] **Step 3: Implement detection functions**

Add to `smpr/src/server/mod.rs`:

```rust
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

    let body_str = response
        .into_body()
        .read_to_string()
        .unwrap_or_default();

    let info: SystemInfoPublic = serde_json::from_str(&body_str).unwrap_or_default();

    detect_from_response(&info, &server_header).ok_or_else(|| {
        MediaServerError::Protocol(format!(
            "cannot determine server type at {clean_url}. \
             Set type = \"emby\" or type = \"jellyfin\" in your TOML config."
        ))
    })
}
```

Update the re-exports at the top of `mod.rs`:

```rust
pub use error::MediaServerError;
pub use types::SystemInfoPublic;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd smpr && cargo test -- server::tests::unit`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add smpr/src/server/mod.rs smpr/src/server/tests/unit.rs
git commit -m "feat(server): add server type auto-detection with 3-tier chain (#69)"
```

---

### Task 5: Integration tests for auto-detection

**Files:**
- Modify: `smpr/src/server/tests/integration.rs`

- [ ] **Step 1: Write UAT-gated integration tests**

Replace `smpr/src/server/tests/integration.rs`:

```rust
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
```

- [ ] **Step 2: Run unit tests (should still pass without UAT)**

Run: `cd smpr && cargo test -- server::tests`
Expected: unit tests pass, integration tests skip (SMPR_UAT_TEST not set)

- [ ] **Step 3: Run integration tests against UAT**

Run: `cd smpr && SMPR_UAT_TEST=1 cargo test -- server::tests::integration`
Expected: all 3 integration tests pass

- [ ] **Step 4: Commit**

```bash
git add smpr/src/server/tests/integration.rs
git commit -m "test(server): add UAT integration tests for auto-detection (#69)"
```

---

### Task 6: PR 1 — verify and create PR

- [ ] **Step 1: Run full test suite**

Run: `cd smpr && cargo test`
Expected: all tests pass, no warnings (except expected `dead_code` allows on config)

- [ ] **Step 2: Run clippy**

Run: `cd smpr && cargo clippy -- -D warnings`
Expected: no errors

- [ ] **Step 3: Run integration tests one final time**

Run: `cd smpr && SMPR_UAT_TEST=1 cargo test -- server::tests::integration`
Expected: all pass

- [ ] **Step 4: Create PR**

Branch: `feat/server-http-core`
Title: `feat(server): HTTP core + auto-detection (#69, #70)`
Body: Summary of what was built, link to issues #69 and #70.
Wait for CodeRabbit before merging.

---

## Chunk 2: PR 2 — User/Item CRUD + Library/Genres (#71 + #73)

### Task 7: Response types for user, items, libraries, genres

**Files:**
- Modify: `smpr/src/server/types.rs`
- Modify: `smpr/src/server/tests/unit.rs`

- [ ] **Step 1: Write failing tests for all new response types**

Add to `smpr/src/server/tests/unit.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd smpr && cargo test -- server::tests::unit::parse_user`
Expected: FAIL — types not defined

- [ ] **Step 3: Implement all new types**

Add to `smpr/src/server/types.rs`:

```rust
use serde_json::Value;

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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd smpr && cargo test -- server::tests::unit`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add smpr/src/server/types.rs smpr/src/server/tests/unit.rs
git commit -m "feat(server): add response types for users, items, libraries, genres (#71, #73)"
```

---

### Task 8: get_user_id, get_item, update_item

**Files:**
- Modify: `smpr/src/server/mod.rs`

- [ ] **Step 1: Implement user ID resolution and item CRUD**

Add methods to `MediaServerClient` in `smpr/src/server/mod.rs`:

```rust
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
```

- [ ] **Step 2: Verify compilation**

Run: `cd smpr && cargo build`
Expected: compiles without errors

- [ ] **Step 3: Commit**

```bash
git add smpr/src/server/mod.rs
git commit -m "feat(server): add get_user_id, get_item, update_item (#71)"
```

---

### Task 9: prefetch_audio_items with pagination

**Files:**
- Modify: `smpr/src/server/mod.rs`
- Modify: `smpr/src/server/tests/unit.rs`

- [ ] **Step 1: Write failing test for pagination item extraction**

Add to `smpr/src/server/tests/unit.rs`:

```rust
use super::super::extract_audio_items;

#[test]
fn extract_items_from_value_array() {
    let json = r#"[
        {"Id": "1", "Path": "/music/a.mp3", "Genres": []},
        {"Id": "2", "Path": "/music/b.mp3", "Genres": ["Rock"]}
    ]"#;
    let items: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd smpr && cargo test -- server::tests::unit::extract_items`
Expected: FAIL — `extract_audio_items` not defined

- [ ] **Step 3: Implement extract_audio_items helper and prefetch_audio_items**

Add to `smpr/src/server/mod.rs`:

```rust
/// Extract (AudioItemView, Value) pairs from raw JSON item values.
/// Skips items that fail to deserialize into AudioItemView.
pub fn extract_audio_items(items: &[Value]) -> Vec<(types::AudioItemView, Value)> {
    items
        .iter()
        .filter_map(|v| {
            let view: types::AudioItemView = serde_json::from_value(v.clone()).ok()?;
            Some((view, v.clone()))
        })
        .collect()
}
```

Add method to `MediaServerClient`:

```rust
    /// Paginated fetch of all audio items. Returns (AudioItemView, raw Value) pairs.
    pub fn prefetch_audio_items(
        &self,
        include_media_sources: bool,
        parent_id: Option<&str>,
    ) -> Result<Vec<(types::AudioItemView, Value)>, MediaServerError> {
        let mut fields = "Path,OfficialRating,AlbumArtist,Album,Genres".to_string();
        if include_media_sources && self.server_type == ServerType::Emby {
            fields.push_str(",MediaSources");
        }
        let uid = self.get_user_id()?;
        let parent_filter = parent_id
            .map(|id| format!("&ParentId={id}"))
            .unwrap_or_default();

        let mut all_items = Vec::new();
        let mut start_index: i64 = 0;
        let page_size = 500;

        loop {
            let path = format!(
                "/Users/{uid}/Items?Recursive=true&IncludeItemTypes=Audio\
                 &Fields={fields}{parent_filter}\
                 &StartIndex={start_index}&Limit={page_size}"
            );
            let result = self.request("GET", &path, None)?;
            let Some(val) = result else {
                // Mid-pagination empty body — return what we have
                if !all_items.is_empty() {
                    log::warn!(
                        "server returned empty body mid-pagination after {} items; \
                         prefetch may be incomplete",
                        all_items.len()
                    );
                }
                break;
            };
            let page: types::PrefetchResponse = serde_json::from_value(val)
                .map_err(|e| MediaServerError::Parse(format!("prefetch response: {e}")))?;
            if page.items.is_empty() {
                break;
            }
            let batch_len = page.items.len() as i64;
            let pairs = extract_audio_items(&page.items);
            all_items.extend(pairs);
            start_index += batch_len;
            log::debug!("fetched {} / {} audio items", start_index, page.total_record_count);
            if start_index >= page.total_record_count {
                break;
            }
        }

        log::info!("prefetched {} audio items from server", all_items.len());
        Ok(all_items)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd smpr && cargo test -- server::tests::unit`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add smpr/src/server/mod.rs smpr/src/server/tests/unit.rs
git commit -m "feat(server): add prefetch_audio_items with pagination (#71)"
```

---

### Task 10: discover_libraries and list_genres

**Files:**
- Modify: `smpr/src/server/mod.rs`

- [ ] **Step 1: Implement discover_libraries and list_genres**

Add methods to `MediaServerClient`:

```rust
    /// GET /Library/VirtualFolders — return music libraries only.
    pub fn discover_libraries(&self) -> Result<Vec<types::VirtualFolder>, MediaServerError> {
        let result = self
            .request("GET", "/Library/VirtualFolders", None)?
            .ok_or_else(|| {
                MediaServerError::Protocol(
                    "empty response from /Library/VirtualFolders".to_string(),
                )
            })?;
        let folders: Vec<types::VirtualFolder> = serde_json::from_value(result)
            .map_err(|e| MediaServerError::Parse(format!("VirtualFolders: {e}")))?;
        let music: Vec<types::VirtualFolder> = folders
            .into_iter()
            .filter(|f| f.collection_type.as_deref() == Some("music"))
            .collect();
        log::info!(
            "discovered {} music library/libraries: {}",
            music.len(),
            music.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", ")
        );
        Ok(music)
    }

    /// GET /MusicGenres?Recursive=true — return sorted genre names.
    pub fn list_genres(&self) -> Result<Vec<String>, MediaServerError> {
        let result = self
            .request("GET", "/MusicGenres?Recursive=true", None)?
            .ok_or_else(|| {
                MediaServerError::Protocol("empty response from /MusicGenres".to_string())
            })?;
        let resp: types::GenreResponse = serde_json::from_value(result)
            .map_err(|e| MediaServerError::Parse(format!("MusicGenres: {e}")))?;
        let mut names: Vec<String> = resp
            .items
            .into_iter()
            .filter(|g| !g.name.is_empty())
            .map(|g| g.name)
            .collect();
        names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        Ok(names)
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cd smpr && cargo build`
Expected: compiles without errors

- [ ] **Step 3: Commit**

```bash
git add smpr/src/server/mod.rs
git commit -m "feat(server): add discover_libraries and list_genres (#73)"
```

---

### Task 11: Integration tests for PR 2

**Files:**
- Modify: `smpr/src/server/tests/integration.rs`

- [ ] **Step 1: Add integration tests for user ID, prefetch, libraries, genres, get_item**

Add to `smpr/src/server/tests/integration.rs`:

```rust
use super::super::MediaServerClient;

fn emby_client() -> Option<MediaServerClient> {
    if !uat_enabled() {
        return None;
    }
    dotenvy::from_path(std::path::Path::new("../.env")).ok();
    let key = std::env::var("EMBY_API_KEY").ok()?;
    Some(MediaServerClient::new(
        "http://localhost:8096".to_string(),
        key,
        ServerType::Emby,
    ))
}

fn jellyfin_client() -> Option<MediaServerClient> {
    if !uat_enabled() {
        return None;
    }
    dotenvy::from_path(std::path::Path::new("../.env")).ok();
    let key = std::env::var("UAT_JELLYFIN_API_KEY").ok()?;
    Some(MediaServerClient::new(
        "http://localhost:8097".to_string(),
        key,
        ServerType::Jellyfin,
    ))
}

#[test]
fn emby_get_user_id() {
    let Some(client) = emby_client() else { return };
    let uid = client.get_user_id().unwrap();
    assert!(!uid.is_empty());
}

#[test]
fn jellyfin_get_user_id() {
    let Some(client) = jellyfin_client() else { return };
    let uid = client.get_user_id().unwrap();
    assert!(!uid.is_empty());
}

#[test]
fn emby_prefetch_audio_items() {
    let Some(client) = emby_client() else { return };
    let items = client.prefetch_audio_items(false, None).unwrap();
    assert!(!items.is_empty(), "expected at least one audio item");
    let (view, raw) = &items[0];
    assert!(!view.id.is_empty());
    assert!(raw.get("Id").is_some());
}

#[test]
fn jellyfin_prefetch_audio_items() {
    let Some(client) = jellyfin_client() else { return };
    let items = client.prefetch_audio_items(false, None).unwrap();
    assert!(!items.is_empty());
}

#[test]
fn emby_discover_libraries() {
    let Some(client) = emby_client() else { return };
    let libs = client.discover_libraries().unwrap();
    assert!(!libs.is_empty(), "expected at least one music library");
    assert_eq!(libs[0].collection_type.as_deref(), Some("music"));
}

#[test]
fn jellyfin_discover_libraries() {
    let Some(client) = jellyfin_client() else { return };
    let libs = client.discover_libraries().unwrap();
    assert!(!libs.is_empty());
}

#[test]
fn emby_list_genres() {
    let Some(client) = emby_client() else { return };
    let genres = client.list_genres().unwrap();
    assert!(!genres.is_empty(), "expected at least one genre");
    // Verify sorted (case-insensitive)
    let sorted: Vec<String> = {
        let mut g = genres.clone();
        g.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        g
    };
    assert_eq!(genres, sorted);
}

#[test]
fn emby_get_item_read_only() {
    let Some(client) = emby_client() else { return };
    let items = client.prefetch_audio_items(false, None).unwrap();
    let (view, _) = &items[0];
    let full_item = client.get_item(&view.id).unwrap();
    assert!(full_item.get("Id").is_some());
    assert!(full_item.get("Name").is_some());
    // Verify round-trip body has more keys than the view
    let keys = full_item.as_object().unwrap().len();
    assert!(keys > 10, "expected full item body, got {keys} keys");
}
```

- [ ] **Step 2: Run integration tests**

Run: `cd smpr && SMPR_UAT_TEST=1 cargo test -- server::tests::integration`
Expected: all tests pass

- [ ] **Step 3: Commit**

```bash
git add smpr/src/server/tests/integration.rs
git commit -m "test(server): add integration tests for CRUD, libraries, genres (#71, #73)"
```

---

### Task 12: PR 2 — verify and create PR

- [ ] **Step 1: Run full test suite + clippy**

Run: `cd smpr && cargo test && cargo clippy -- -D warnings`
Expected: all pass

- [ ] **Step 2: Run integration tests**

Run: `cd smpr && SMPR_UAT_TEST=1 cargo test -- server::tests::integration`
Expected: all pass

- [ ] **Step 3: Create PR**

Branch: `feat/server-crud-libraries`
Title: `feat(server): user/item CRUD + library/genre discovery (#71, #73)`
Body: Summary of what was built, link to issues #71 and #73. Depends on PR 1.
Wait for CodeRabbit before merging.

---

## Chunk 3: PR 3 — Lyrics + Authenticate by Name (#72 + #74)

### Task 13: strip_lrc_tags utility

**Files:**
- Create: `smpr/src/util.rs`
- Modify: `smpr/src/main.rs`

- [ ] **Step 1: Write failing tests for strip_lrc_tags**

Create `smpr/src/util.rs` with tests only:

```rust
use regex::Regex;

// Implementation will go here

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_timestamps() {
        let input = "[00:15.30]Hello world\n[00:20.00]Second line";
        let result = strip_lrc_tags(input);
        assert_eq!(result, "Hello world\nSecond line");
    }

    #[test]
    fn strip_metadata_lines() {
        let input = "[ar:Artist Name]\n[ti:Song Title]\nActual lyrics here";
        let result = strip_lrc_tags(input);
        assert!(result.contains("Actual lyrics here"));
        assert!(!result.contains("[ar:"));
        assert!(!result.contains("[ti:"));
    }

    #[test]
    fn passthrough_plain_text() {
        let input = "Just plain text lyrics\nNo tags at all";
        let result = strip_lrc_tags(input);
        assert_eq!(result, input);
    }

    #[test]
    fn empty_input() {
        assert_eq!(strip_lrc_tags(""), "");
    }

    #[test]
    fn mixed_timestamps_and_text() {
        let input = "[01:23.45]Line one\nPlain line\n[02:00.00]Line three";
        let result = strip_lrc_tags(input);
        assert_eq!(result, "Line one\nPlain line\nLine three");
    }
}
```

- [ ] **Step 2: Add `mod util;` to main.rs**

Add `mod util;` to the module declarations in `smpr/src/main.rs` (after `mod tui;`).

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd smpr && cargo test -- util::tests`
Expected: FAIL — `strip_lrc_tags` not defined

- [ ] **Step 4: Implement strip_lrc_tags**

Add to `smpr/src/util.rs` (before the `#[cfg(test)]` block):

```rust
use regex::Regex;
use std::sync::LazyLock;

static LRC_TIMESTAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\d{1,3}:\d{2}(?:\.\d{1,3})?\]").unwrap());

static LRC_METADATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?mi)^\[[a-z]{2,}:.*\]$").unwrap());

/// Remove LRC timestamp tags and metadata lines from lyrics text.
pub fn strip_lrc_tags(text: &str) -> String {
    let text = LRC_TIMESTAMP.replace_all(text, "");
    let text = LRC_METADATA.replace_all(&text, "");
    text.to_string()
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd smpr && cargo test -- util::tests`
Expected: all 5 tests pass

- [ ] **Step 6: Commit**

```bash
git add smpr/src/util.rs smpr/src/main.rs
git commit -m "feat(util): add strip_lrc_tags for LRC text normalization (#72)"
```

---

### Task 14: Lyrics response types

**Files:**
- Modify: `smpr/src/server/types.rs`
- Modify: `smpr/src/server/tests/unit.rs`

- [ ] **Step 1: Write failing tests for lyrics types**

Add to `smpr/src/server/tests/unit.rs`:

```rust
use super::super::types::LyricsResponse;

#[test]
fn parse_jellyfin_lyrics_response() {
    let json = r#"{
        "Lyrics": [
            {"Text": "Hello world", "Start": 0},
            {"Text": "Second line", "Start": 5000000}
        ]
    }"#;
    let resp: LyricsResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.lyrics.len(), 2);
    assert_eq!(resp.lyrics[0].text.as_deref(), Some("Hello world"));
}

#[test]
fn parse_jellyfin_lyrics_empty() {
    let json = r#"{"Lyrics": []}"#;
    let resp: LyricsResponse = serde_json::from_str(json).unwrap();
    assert!(resp.lyrics.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd smpr && cargo test -- server::tests::unit::parse_jellyfin_lyrics`
Expected: FAIL — `LyricsResponse` not defined

- [ ] **Step 3: Implement lyrics types**

Add to `smpr/src/server/types.rs`:

```rust
/// Response from GET /Audio/{id}/Lyrics (Jellyfin only).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LyricsResponse {
    #[serde(default)]
    pub lyrics: Vec<LyricLine>,
}

/// Single lyric line in a Jellyfin lyrics response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LyricLine {
    pub text: Option<String>,
    pub start: Option<i64>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd smpr && cargo test -- server::tests::unit::parse_jellyfin_lyrics`
Expected: 2 tests pass

- [ ] **Step 5: Commit**

```bash
git add smpr/src/server/types.rs smpr/src/server/tests/unit.rs
git commit -m "feat(server): add LyricsResponse and LyricLine types (#72)"
```

---

### Task 15: fetch_lyrics implementation

**Files:**
- Modify: `smpr/src/server/mod.rs`
- Modify: `smpr/src/server/tests/unit.rs`

- [ ] **Step 1: Write failing tests for Emby lyrics extraction from canned MediaSources**

Add to `smpr/src/server/tests/unit.rs`:

```rust
use super::super::find_emby_lyrics_stream;

#[test]
fn emby_find_external_lrc_stream() {
    let raw = serde_json::json!({
        "Id": "8177",
        "MediaSources": [{
            "Id": "mediasource_8177",
            "MediaStreams": [
                {"Codec": "mp3", "Type": "Audio", "Index": 0, "IsExternal": false},
                {"Codec": "lrc", "Type": "Subtitle", "Index": 1, "IsExternal": true,
                 "Path": "/music/test.lrc"}
            ]
        }]
    });
    let result = find_emby_lyrics_stream(&raw);
    assert!(result.is_some());
    let (media_source_id, stream_index) = result.unwrap();
    assert_eq!(media_source_id, "mediasource_8177");
    assert_eq!(stream_index, 1);
}

#[test]
fn emby_no_subtitle_streams() {
    let raw = serde_json::json!({
        "Id": "1234",
        "MediaSources": [{
            "Id": "ms_1234",
            "MediaStreams": [
                {"Codec": "mp3", "Type": "Audio", "Index": 0, "IsExternal": false}
            ]
        }]
    });
    assert!(find_emby_lyrics_stream(&raw).is_none());
}

#[test]
fn emby_internal_subtitle_not_matched() {
    let raw = serde_json::json!({
        "Id": "1234",
        "MediaSources": [{
            "Id": "ms_1234",
            "MediaStreams": [
                {"Codec": "lrc", "Type": "Subtitle", "Index": 1, "IsExternal": false,
                 "Extradata": "embedded lyrics text"}
            ]
        }]
    });
    // find_emby_lyrics_stream only looks for external streams
    assert!(find_emby_lyrics_stream(&raw).is_none());
}

#[test]
fn emby_non_lrc_external_subtitle_skipped() {
    let raw = serde_json::json!({
        "Id": "1234",
        "MediaSources": [{
            "Id": "ms_1234",
            "MediaStreams": [
                {"Codec": "srt", "Type": "Subtitle", "Index": 1, "IsExternal": true}
            ]
        }]
    });
    assert!(find_emby_lyrics_stream(&raw).is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd smpr && cargo test -- server::tests::unit::emby_find`
Expected: FAIL — `find_emby_lyrics_stream` not defined

- [ ] **Step 3: Implement lyrics helpers and fetch_lyrics**

Add to `smpr/src/server/mod.rs`:

```rust
use crate::util::strip_lrc_tags;

/// Find the first external LRC subtitle stream in an Emby item's MediaSources.
/// Returns (media_source_id, stream_index) if found.
pub fn find_emby_lyrics_stream(raw: &Value) -> Option<(String, i64)> {
    let sources = raw.get("MediaSources")?.as_array()?;
    for source in sources {
        let Some(media_source_id) = source.get("Id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(streams) = source.get("MediaStreams").and_then(|v| v.as_array()) else {
            continue;
        };
        for stream in streams {
            if stream.get("Type")?.as_str()? != "Subtitle" {
                continue;
            }
            if !stream.get("IsExternal").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
            let codec = stream.get("Codec").and_then(|v| v.as_str()).unwrap_or("");
            if !codec.eq_ignore_ascii_case("lrc") {
                continue;
            }
            let index = stream.get("Index")?.as_i64()?;
            return Some((media_source_id.to_string(), index));
        }
    }
    None
}

/// Extract embedded lyrics from Extradata on internal subtitle streams.
fn extract_embedded_lyrics(raw: &Value) -> Option<String> {
    let sources = raw.get("MediaSources")?.as_array()?;
    let mut fragments = Vec::new();
    for source in sources {
        let Some(streams) = source.get("MediaStreams").and_then(|v| v.as_array()) else {
            continue;
        };
        for stream in streams {
            if stream.get("IsExternal").and_then(|v| v.as_bool()).unwrap_or(true) {
                continue;
            }
            if stream.get("Type").and_then(|v| v.as_str()) != Some("Subtitle") {
                continue;
            }
            if let Some(extradata) = stream.get("Extradata").and_then(|v| v.as_str()) {
                let trimmed = extradata.trim();
                if !trimmed.is_empty() {
                    fragments.push(trimmed.to_string());
                }
            }
        }
    }
    if fragments.is_empty() {
        return None;
    }
    Some(strip_lrc_tags(&fragments.join("\n")))
}
```

Add method to `MediaServerClient`:

```rust
    /// Fetch lyrics for an audio item, abstracting server-specific logic.
    /// Returns Ok(Some(text)) with plain lyrics or Ok(None) if no lyrics found.
    pub fn fetch_lyrics(
        &self,
        item: &types::AudioItemView,
        raw: &Value,
    ) -> Result<Option<String>, MediaServerError> {
        match self.server_type {
            ServerType::Emby => self.fetch_lyrics_emby(item, raw),
            ServerType::Jellyfin => self.fetch_lyrics_jellyfin(&item.id),
        }
    }

    fn fetch_lyrics_emby(
        &self,
        item: &types::AudioItemView,
        raw: &Value,
    ) -> Result<Option<String>, MediaServerError> {
        // Try external subtitle stream first
        if let Some((media_source_id, stream_index)) = find_emby_lyrics_stream(raw) {
            let path = format!(
                "/Videos/{}/{}/Subtitles/{}/Stream.txt",
                item.id, media_source_id, stream_index
            );
            match self.request_text("GET", &path) {
                Ok(text) => {
                    let cleaned = strip_lrc_tags(&text);
                    if !cleaned.trim().is_empty() {
                        return Ok(Some(cleaned));
                    }
                }
                Err(MediaServerError::Http { status, .. }) if status == 401 || status == 403 => {
                    return Err(MediaServerError::Http {
                        status,
                        body: format!("auth error fetching lyrics for {}", item.id),
                    });
                }
                Err(e) => {
                    log::warn!("Emby subtitle fetch failed for {} (stream {}): {e}",
                        item.path.as_deref().unwrap_or("<unknown>"), stream_index);
                }
            }
        }

        // Fallback: embedded lyrics from Extradata
        Ok(extract_embedded_lyrics(raw))
    }

    fn fetch_lyrics_jellyfin(
        &self,
        item_id: &str,
    ) -> Result<Option<String>, MediaServerError> {
        let path = format!("/Audio/{item_id}/Lyrics");
        match self.request("GET", &path, None) {
            Ok(Some(val)) => {
                let resp: types::LyricsResponse = serde_json::from_value(val)
                    .map_err(|e| MediaServerError::Parse(format!("lyrics response: {e}")))?;
                let lines: Vec<&str> = resp
                    .lyrics
                    .iter()
                    .filter_map(|l| l.text.as_deref())
                    .filter(|t| !t.is_empty())
                    .collect();
                if lines.is_empty() {
                    return Ok(None);
                }
                let text = strip_lrc_tags(&lines.join("\n"));
                if text.trim().is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(text))
                }
            }
            Ok(None) => Ok(None),
            Err(MediaServerError::Http { status, .. }) if status == 401 || status == 403 => {
                Err(MediaServerError::Http {
                    status,
                    body: format!("auth error fetching lyrics for {item_id}"),
                })
            }
            Err(MediaServerError::Http { status: 404, .. }) => Ok(None),
            Err(e) => {
                log::warn!("Jellyfin lyrics fetch failed for {item_id}: {e}");
                Ok(None)
            }
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd smpr && cargo test -- server::tests::unit`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add smpr/src/server/mod.rs smpr/src/server/tests/unit.rs
git commit -m "feat(server): add fetch_lyrics for Emby and Jellyfin (#72)"
```

---

### Task 16: authenticate_by_name

**Files:**
- Modify: `smpr/src/server/mod.rs`

- [ ] **Step 1: Implement authenticate_by_name standalone function**

Add to `smpr/src/server/mod.rs`:

```rust
/// Authenticate via POST /Users/AuthenticateByName.
/// Standalone function — no client instance needed (called before API key exists).
/// Returns the AccessToken from the response.
pub fn authenticate_by_name(
    url: &str,
    username: &str,
    password: &str,
) -> Result<String, MediaServerError> {
    let clean_url = url.trim_end_matches('/');
    let endpoint = format!("{clean_url}/Users/AuthenticateByName");

    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let device_id = uuid::Uuid::new_v4().to_string();
    let version = env!("CARGO_PKG_VERSION");

    let auth_header = format!(
        "MediaBrowser Client=\"smpr\", Device=\"{hostname}\", \
         DeviceId=\"{device_id}\", Version=\"{version}\""
    );

    let body = serde_json::json!({
        "Username": username,
        "Pw": password,
    });

    let agent = ureq::Agent::config_builder()
        .timeout_per_call(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .new_agent();

    let response = agent
        .post(&endpoint)
        .header("X-Emby-Authorization", &auth_header)
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| {
            MediaServerError::Connection(format!(
                "cannot reach {endpoint} for authentication: {e}"
            ))
        })?;

    let status = response.status().as_u16();
    if status >= 400 {
        let body_snippet = response
            .into_body()
            .read_to_string()
            .unwrap_or_default();
        return Err(MediaServerError::Http {
            status,
            body: format!("authentication failed: {body_snippet}"),
        });
    }

    let body_str = response
        .into_body()
        .read_to_string()
        .map_err(|e| MediaServerError::Connection(format!("read error: {e}")))?;
    let val: Value = serde_json::from_str(&body_str)
        .map_err(|e| MediaServerError::Parse(format!("auth response: {e}")))?;

    val.get("AccessToken")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            MediaServerError::Protocol(
                "authentication response missing AccessToken".to_string(),
            )
        })
}
```

- [ ] **Step 2: Add `hostname` and `uuid` dependencies**

Run: `cd smpr && cargo add hostname uuid --features uuid/v4`

- [ ] **Step 3: Verify compilation**

Run: `cd smpr && cargo build`
Expected: compiles without errors

- [ ] **Step 4: Commit**

```bash
git add smpr/src/server/mod.rs smpr/Cargo.toml smpr/Cargo.lock
git commit -m "feat(server): add authenticate_by_name for configure wizard (#74)"
```

---

### Task 17: Integration tests for lyrics

**Files:**
- Modify: `smpr/src/server/tests/integration.rs`

- [ ] **Step 1: Add integration tests for lyrics fetch**

Add to `smpr/src/server/tests/integration.rs`:

```rust
use super::super::types::AudioItemView;

#[test]
fn emby_fetch_lyrics_known_lrc() {
    let Some(client) = emby_client() else { return };
    // Item 8177 has a known LRC sidecar
    let items = client.prefetch_audio_items(true, None).unwrap();
    let target = items.iter().find(|(v, _)| v.id == "8177");
    if let Some((view, raw)) = target {
        let lyrics = client.fetch_lyrics(view, raw).unwrap();
        assert!(lyrics.is_some(), "expected lyrics for item 8177");
        let text = lyrics.unwrap();
        assert!(!text.trim().is_empty(), "expected non-empty lyrics text");
    } else {
        eprintln!("item 8177 not found in prefetch; skipping lyrics test");
    }
}

#[test]
fn jellyfin_fetch_lyrics_graceful_none() {
    let Some(client) = jellyfin_client() else { return };
    let items = client.prefetch_audio_items(false, None).unwrap();
    if let Some((view, raw)) = items.first() {
        // Most items won't have lyrics — verify graceful None
        let result = client.fetch_lyrics(view, raw);
        assert!(result.is_ok(), "lyrics fetch should not error: {:?}", result.err());
    }
}
```

- [ ] **Step 2: Run integration tests**

Run: `cd smpr && SMPR_UAT_TEST=1 cargo test -- server::tests::integration`
Expected: all tests pass

- [ ] **Step 3: Commit**

```bash
git add smpr/src/server/tests/integration.rs
git commit -m "test(server): add integration tests for lyrics fetch (#72)"
```

---

### Task 18: PR 3 — verify and create PR

- [ ] **Step 1: Run full test suite + clippy**

Run: `cd smpr && cargo test && cargo clippy -- -D warnings`
Expected: all pass

- [ ] **Step 2: Run integration tests**

Run: `cd smpr && SMPR_UAT_TEST=1 cargo test -- server::tests::integration`
Expected: all pass

- [ ] **Step 3: Create PR**

Branch: `feat/server-lyrics-auth`
Title: `feat(server): lyrics fetch + authenticate_by_name (#72, #74)`
Body: Summary of what was built, link to issues #72 and #74. Depends on PR 2.
Wait for CodeRabbit before merging.

---

## Post-Milestone

After all 3 PRs are merged:
- Close issues #69, #70, #71, #72, #73, #74
- The server module is complete — future milestones (detection engine, rating orchestration, configure wizard) consume its public API
