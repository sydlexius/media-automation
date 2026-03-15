# Server API Client — Design Spec

**Date:** 2026-03-15
**Status:** Draft
**Scope:** Issues #69–#74 (Milestone: Rust Rewrite — Server API Client)
**Depends on:** Config loading (#67–#68), merged on main

---

## Problem

The Rust rewrite has config loading and CLI parsing but no server communication.
The `server.rs` file is a stub. All six issues in this milestone port the Python
`MediaServerClient` class and related standalone functions to Rust, providing the
HTTP client, server type detection, item CRUD, lyrics fetching, library discovery,
and authentication needed by future milestones (detection, rating, configure wizard).

## Approach

Port the Python `MediaServerClient` to an idiomatic Rust module using `ureq` for
blocking HTTP. Use a hybrid typing strategy validated against live Emby and
Jellyfin API responses: typed structs for read-only endpoints, `serde_json::Value`
for the item round-trip body. Three PRs, merged sequentially.

---

## Module Structure

```
smpr/src/server/
├── mod.rs             # MediaServerClient struct, public API, re-exports
├── error.rs           # MediaServerError enum
├── types.rs           # Typed response structs
└── tests/
    ├── mod.rs
    ├── unit.rs        # Canned JSON parsing tests
    └── integration.rs # UAT-only tests (gated behind env var)
```

`src/main.rs` updates `mod server` from a single file to a directory module.

---

## Error Type (`error.rs`)

`MediaServerError` covers all failure modes:

- **Http** — status code + response body snippet (mirrors Python's `status_code` field)
- **Connection** — server unreachable or timeout
- **Parse** — response body is not valid JSON when expected
- **Protocol** — valid JSON but missing expected fields (e.g. `/Users` returns empty list)

All methods on `MediaServerClient` return `Result<T, MediaServerError>`. The caller
(rating orchestration, future milestone) decides which errors are fatal vs.
recoverable. The server module reports errors without swallowing them.

Implements `Display`, `std::error::Error`, and `From<ureq::Error>` for
ergonomic `?` propagation from `ureq` calls.

---

## Typed Response Structs (`types.rs`)

Validated against live responses from UAT Emby (4.9.3.0), prod Emby (4.10.0.5),
and UAT Jellyfin (10.11.6). Both servers use PascalCase field names consistently.

**All response structs** use `#[serde(rename_all = "PascalCase")]` to map
Rust's `snake_case` fields to the API's PascalCase keys. Unknown fields are
silently ignored (no `deny_unknown_fields`).

### `/System/Info/Public` — auto-detection (#69)

```
SystemInfoPublic
  product_name: Option<String>    // Present on Jellyfin, absent on Emby
  server_name: Option<String>
  version: Option<String>
  id: Option<String>
  local_address: Option<String>   // Jellyfin: singular string
  local_addresses: Option<Vec<String>>  // Emby: plural array
  startup_wizard_completed: Option<bool>  // Jellyfin-only
```

Deserialize with `#[serde(default)]` on all fields so both server shapes parse
into the same struct without error. The detection function inspects which fields
are populated.

### `/Users` — user ID resolution (#71)

```
UserInfo
  id: String
  name: Option<String>
```

Only the first user's ID is needed.

### `/Library/VirtualFolders` — library discovery (#73)

```
VirtualFolder
  name: String
  item_id: String
  collection_type: Option<String>
  locations: Vec<String>
```

Both servers return the same shape. Filtered for `collection_type == "music"`.

### `/MusicGenres` — genre listing (#73)

```
GenreItem
  name: String

GenreResponse
  items: Vec<GenreItem>
```

### `/Audio/{id}/Lyrics` — Jellyfin lyrics (#72)

```
LyricsResponse
  lyrics: Vec<LyricLine>

LyricLine
  text: Option<String>
  start: Option<i64>
```

### Audio items from prefetch (#71)

```
AudioItemView (read-only view — deserialized alongside the raw Value)
  id: String
  path: Option<String>
  official_rating: Option<String>
  album_artist: Option<String>
  album: Option<String>
  genres: Vec<String>

PrefetchResponse
  items: Vec<Value>
  total_record_count: i64
```

Items are keyed by item ID, not path (departure from Python). The typed
`AudioItemView` is deserialized from the same JSON as the raw `Value` — one
parse, two targets. The `Value` is kept for the round-trip update.

### Round-trip item body (#71)

Stored as `serde_json::Value`. Read fields through `AudioItemView`, mutate only
`OfficialRating` on the raw `Value` before `POST /Items/{id}`.

**Why hybrid typing:** The full item body has 49 keys (Emby) / 51 keys
(Jellyfin), with significant divergence between servers. Typing this would lose
unknown fields on re-serialization, corrupting server data. The untyped surface
area is exactly one mutation: setting `OfficialRating`.

---

## Server Type Auto-Detection (#69)

### Detection chain

Three tiers of signals, validated against live servers and developer documentation:

1. **ProductName (official identification):** `ProductName == "Jellyfin Server"`
   → Jellyfin. `ProductName` present but anything else → Emby. This field was
   [intentionally added](https://github.com/jellyfin/jellyfin/issues/509) by
   Jellyfin maintainers specifically for client identification.

2. **Structural shape (fallback):** `local_address` key present (singular) →
   Jellyfin. `local_addresses` key present (plural) → Emby. This is a structural
   API difference between the two products' `PublicSystemInfo` models. If both
   keys are present (e.g. a future version adds the other form for compatibility),
   `local_address` (singular, Jellyfin-specific) takes precedence.

3. **Server header (network fallback):** `Server` response header contains
   `Kestrel` → Jellyfin. This can be unreliable behind reverse proxies.

4. **Manual override:** If no signal matches, return error with guidance to set
   `type = "emby"` or `type = "jellyfin"` in TOML config.

### TOML override

If `type` is specified in `[servers.*]`, auto-detection is skipped entirely.
This is the escape hatch for edge cases (reverse proxy stripping headers,
non-standard server builds).

### Evidence from live servers

| Signal | UAT Emby 4.9.3.0 | Prod Emby 4.10.0.5 | UAT Jellyfin 10.11.6 |
|--------|-------------------|---------------------|----------------------|
| `ProductName` | Absent | Absent | `"Jellyfin Server"` |
| `LocalAddress` (singular) | Absent | Absent | Present |
| `LocalAddresses` (plural) | Present | Present | Absent |
| `Server` header | `UPnP/1.0 DLNADOC/1.50` | `UPnP/1.0 DLNADOC/1.50` | `Kestrel` |

### Scope limitation

Detection is scoped to Emby and Jellyfin only. If a third server product (e.g.
Plex) is ever added, the detection function would need to become a proper
fingerprint matcher rather than a binary Jellyfin-or-not check. This is
documented as an explicit non-goal for now.

### Standalone function

`detect_server_type(url: &str) -> Result<ServerType, MediaServerError>`

Called before `MediaServerClient` construction. Unauthenticated — no API key
needed. 10-second timeout.

---

## MediaServerClient Struct

```
MediaServerClient
  base_url: String           // trailing slash stripped
  api_key: String
  server_type: ServerType    // resolved before client construction (required)
  user_id: OnceCell<String>  // lazily cached, interior mutability via OnceCell
```

**Interior mutability for `user_id`:** Using `OnceCell<String>` (from `std::cell`)
allows `get_user_id` to cache the user ID through a `&self` reference. This
avoids requiring `&mut self` on every method that needs the user ID (`get_item`,
`prefetch_audio_items`, `fetch_lyrics`, etc.), which would create borrow-checker
friction for callers holding multiple references.

**Construction flow:** Config loading produces `ServerConfig` with
`server_type: Option<ServerType>`. The caller runs `detect_server_type(url)` to
fill in `None` values, then constructs `MediaServerClient` with a known
`ServerType`. The client does not accept `Option<ServerType>`.

### Auth headers

- **Emby:** `X-Emby-Token: {api_key}`
- **Jellyfin:** `X-MediaBrowser-Token: {api_key}`

Both send `Content-Type: application/json` and `Accept: application/json` on
JSON requests. The text endpoint (`request_text`) skips `Content-Type`.

### Timeouts

- Auto-detection: 10 seconds
- Authenticated requests: 15 seconds

---

## Internal Methods

### `request(&self, method, path, body?) -> Result<Option<Value>, MediaServerError>`

Authenticated JSON request. Adds auth header based on `server_type`. Parses
response as `serde_json::Value`. Returns `Ok(None)` when the response body is
empty (e.g. `POST /Items/{id}` returns no content). Returns
`MediaServerError::Http` with status code and body snippet on HTTP errors.

### `request_text(&self, method, path) -> Result<String, MediaServerError>`

Authenticated plain-text request. Same auth header logic, returns raw response
body as `String`. Used for Emby subtitle stream endpoint.

---

## Public Methods

### User ID resolution (#71)

**`get_user_id(&self) -> Result<&str, MediaServerError>`**

`GET /Users` → deserialize as `Vec<UserInfo>` → cache first user's `id` in
`OnceCell`. Returns cached value on subsequent calls. Errors if list is empty
or first user has no ID.

### Item CRUD (#71)

**`get_item(&self, item_id: &str) -> Result<Value, MediaServerError>`**

`GET /Users/{uid}/Items/{id}` → returns full raw JSON `Value` for round-trip
update.

**`update_item(&self, item_id: &str, body: &Value) -> Result<(), MediaServerError>`**

`POST /Items/{id}` with full JSON body. Used to set `OfficialRating`.

### Prefetch (#71)

**`prefetch_audio_items(&self, include_media_sources: bool, parent_id: Option<&str>) -> Result<Vec<(AudioItemView, Value)>, MediaServerError>`**

Paginated `GET /Users/{uid}/Items?Recursive=true&IncludeItemTypes=Audio&Fields=...`.
Pages of 500 items. `include_media_sources` appends `MediaSources` to the
`Fields` parameter (Emby only — needed for lyrics stream discovery).
`parent_id` scopes to a specific library.

**Pagination termination:** Stops when `StartIndex >= TotalRecordCount` or when
the server returns an empty `Items` batch. Mid-pagination empty body (non-JSON
or null response) logs a warning and returns the items collected so far.

Returns pairs of `(AudioItemView, Value)` — typed view for reading fields, raw
JSON for round-trip updates. Items are collected into a `Vec` keyed by item ID.

### Library discovery (#73)

**`discover_libraries(&self) -> Result<Vec<VirtualFolder>, MediaServerError>`**

`GET /Library/VirtualFolders` → filters to `collection_type == "music"`.

### Genre listing (#73)

**`list_genres(&self) -> Result<Vec<String>, MediaServerError>`**

`GET /MusicGenres?Recursive=true` → returns sorted genre names.

### Lyrics fetch (#72)

**`fetch_lyrics(&self, item: &AudioItemView, raw: &Value) -> Result<Option<String>, MediaServerError>`**

Dispatches to Emby or Jellyfin path based on `server_type`. Returns
`Ok(Some(text))` with plain text lyrics (LRC-stripped) or `Ok(None)` if the
track has no lyrics.

**Emby path:** Traverses `MediaSources[].MediaStreams[]` in the raw `Value`,
looking for external subtitle streams (`Type=Subtitle, Codec=lrc, IsExternal=true`).
Fetches via `GET /Videos/{itemId}/{mediaSourceId}/Subtitles/{streamIndex}/Stream.txt`.
Falls back to embedded lyrics from `Extradata` on internal subtitle streams.

**Jellyfin path:** `GET /Audio/{itemId}/Lyrics` → deserializes as
`LyricsResponse` → extracts `Text` fields → joins with newlines.

Both paths apply `strip_lrc_tags` defensively before returning.

**`strip_lrc_tags` location:** This function is needed by `fetch_lyrics` in PR 3
but logically belongs with text processing. It is introduced in PR 3 as
`src/util.rs` (a shared utility module) so it can be consumed by both the server
module now and the detection module later. `src/main.rs` adds `mod util`.

### Authenticate by name (#74)

**`authenticate_by_name(url: &str, username: &str, password: &str) -> Result<String, MediaServerError>`**

Standalone function (no client instance — called before API key exists).

**Note:** This is a new function, not a port from Python. The Python codebase
does not implement `authenticate_by_name`. The auth header format is documented
in both the [Emby API wiki](https://github.com/MediaBrowser/Emby/wiki/Api-Key-Authentication)
and Jellyfin's client SDK. Jellyfin accepts `X-Emby-Authorization` (fork
heritage — never renamed).

`POST /Users/AuthenticateByName` with body `{"Username": "...", "Pw": "..."}`.

Auth header (works for both Emby and Jellyfin):
```
X-Emby-Authorization: MediaBrowser Client="smpr", Device="{hostname}", DeviceId="{uuid}", Version="{cargo_version}"
```

Returns the `AccessToken` from the response. Used by the configure wizard to
obtain an API key from username/password credentials.

Not integration-tested (would create a device entry on UAT servers).

---

## Testing Strategy

### Unit tests (`tests/unit.rs`)

Canned JSON, no network. Run in CI.

- **Auto-detection parsing:** Known JSON blobs for each tier of the detection
  chain (ProductName present/absent, structural fallback, header fallback).
  Verify correct `ServerType` resolution.
- **Response deserialization:** Canned JSON for each typed struct
  (`SystemInfoPublic`, `VirtualFolder`, `GenreResponse`, `LyricsResponse`,
  `AudioItemView`, `UserInfo`). Verify fields parse correctly, unknown fields
  ignored.
- **Pagination logic:** Simulate multi-page responses. Verify all items
  collected. Edge cases: empty batch, mid-pagination empty body.
- **Auth header selection:** Verify `X-Emby-Token` for Emby,
  `X-MediaBrowser-Token` for Jellyfin.
- **Error mapping:** HTTP 404, 401, connection timeout, malformed JSON all
  produce the correct `MediaServerError` variant.
- **Emby lyrics traversal:** Canned `MediaSources` JSON with various stream
  configurations (external LRC, embedded Extradata, no lyrics, missing Index).
  Verify correct stream selection and fallback.

### Integration tests (`tests/integration.rs`)

UAT servers only. Gated behind `SMPR_UAT_TEST=1` env var. Hard-coded to
`localhost:8096` (Emby) and `localhost:8097` (Jellyfin). API keys loaded via
`dotenvy` (already a dependency) from `.env` at repo root: `EMBY_API_KEY` and
`UAT_JELLYFIN_API_KEY`. **Read-only — no mutations to UAT data.**

- Auto-detection end-to-end against both UAT servers
- User ID resolution returns non-empty string
- Prefetch one page of audio items, verify `AudioItemView` fields populated
- Library discovery returns at least one music library
- Genre listing returns non-empty sorted list
- Emby lyrics for item 8177 (known LRC sidecar) returns non-empty text
- Jellyfin lyrics attempt returns graceful `None` for items without lyrics
- `get_item` returns raw `Value` with expected keys (no update call)
- `authenticate_by_name` is **not** tested (would create device entry)

---

## PR Breakdown

### PR 1: HTTP core + auto-detection (#70 + #69)

**Files:**
- `src/server/mod.rs` — `MediaServerClient` struct, `request()`,
  `request_text()`, `detect_server_type()`
- `src/server/error.rs` — `MediaServerError`
- `src/server/types.rs` — `SystemInfoPublic` only
- `src/server/tests/unit.rs` — detection parsing, auth headers, error mapping
- `src/server/tests/integration.rs` — detection against both UAT servers
- `src/main.rs` — update module declaration

Self-contained and testable. `detect_server_type` exists and is tested; wiring
into the main flow is optional for this PR.

### PR 2: User/item CRUD + library/genres (#71 + #73)

**Files modified:**
- `src/server/mod.rs` — `get_user_id()`, `get_item()`, `update_item()`,
  `prefetch_audio_items()`, `discover_libraries()`, `list_genres()`
- `src/server/types.rs` — `UserInfo`, `AudioItemView`, `VirtualFolder`,
  `GenreItem`, `GenreResponse`, `PrefetchResponse`
- `src/server/tests/unit.rs` — pagination, deserialization for all new types
- `src/server/tests/integration.rs` — user ID, prefetch, libraries, genres,
  get_item read-only

Depends on PR 1. Delivers the full "read the server" capability.

### PR 3: Lyrics + authenticate_by_name (#72 + #74)

**Files created/modified:**
- `src/util.rs` — `strip_lrc_tags()` (shared utility, new file)
- `src/server/mod.rs` — `fetch_lyrics()`, `authenticate_by_name()`
- `src/server/types.rs` — `LyricsResponse`, `LyricLine`
- `src/server/tests/unit.rs` — Emby MediaSources traversal, Jellyfin lyrics
  parsing, auth header construction, `strip_lrc_tags` tests
- `src/server/tests/integration.rs` — Emby lyrics for item 8177, Jellyfin
  lyrics attempt
- `src/main.rs` — add `mod util`

Depends on PR 2. Completes the Server API Client milestone.

### Merge protocol

Each PR waits for CodeRabbit to finish reviewing before merging.

---

## What's NOT in scope

- Detection engine (Milestone: Detection)
- Rating orchestration / `process_library` (Milestone: Rating)
- Configure wizard TUI (Milestone: Configure Wizard)
- `_resolve_library_scope` / `_filter_by_location` (Rating milestone — consumes
  `discover_libraries` from this milestone but adds scoping logic)
- `strip_lrc_tags` is introduced in PR 3 as `src/util.rs` (shared utility
  module); consumed by server module and later by detection module

---

## References

- Python source: `SetMusicParentalRating/SetMusicParentalRating.py` — `MediaServerClient` class (line 645), `detect_server_type` (line 216)
- API-driven refactor design: `docs/superpowers/specs/2026-03-13-api-driven-refactor-design.md`
- Config spec: `docs/superpowers/specs/2026-03-14-config-and-cli-design.md`
- [Jellyfin PublicSystemInfo source](https://github.com/jellyfin/jellyfin/blob/master/MediaBrowser.Model/System/PublicSystemInfo.cs)
- [Jellyfin Issue #509 — ProductName added for client identification](https://github.com/jellyfin/jellyfin/issues/509)
- [Emby REST API docs](https://dev.emby.media/doc/restapi/index.html)
