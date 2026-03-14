# API-Driven Refactor — Design Spec

**Date:** 2026-03-13
**Status:** Approved
**Scope:** Replace filesystem-based lyrics scanning with API-driven lyrics fetching, restructure CLI subcommands, add named server model and configure wizard.

---

## Problem

The script scans the local filesystem for sidecar lyric files, then matches them against server items by path. When the media server runs in Docker, the filesystem paths differ from the server's paths (e.g. host has `/mnt/user/music`, container sees `/share/music`). This path mismatch makes the script unusable with containerized servers without a translation layer.

Both Emby and Jellyfin already serve lyrics (sidecar and embedded) via their APIs. The filesystem scan is unnecessary.

## Solution

Eliminate filesystem access entirely. Fetch lyrics from the server API, classify them, and set ratings. The server becomes the single source of truth for track metadata and lyrics content.

---

## Architecture

### Data Flow (New)

1. Connect to server, auto-detect type via `GET /System/Info/Public`
2. Discover music libraries via `GET /Library/VirtualFolders` (filter `CollectionType == "music"`)
3. Prefetch all audio items (existing bulk endpoint, scoped by library/location), keyed by item ID (not path)
4. For each item, fetch lyrics:
   - **Emby:** Try sidecar first (`MediaStreams` with `Type=Subtitle, Codec=lrc, IsExternal=true` → fetch via subtitle endpoint), fall back to embedded (`Extradata`). Stop at first non-empty result.
   - **Jellyfin:** `GET /Audio/{id}/Lyrics` → extract `Text` fields from JSON response
5. Normalize lyrics text via `strip_lrc_tags` (defensive — some endpoints may return raw LRC content)
6. Classify lyrics text (existing `classify_lyrics` — unchanged)
7. For items with no lyrics, check genres against allow-list → set G if matched
8. Set/skip rating via existing `_decide_rating_action`

### What Gets Removed

**Functions:**
- `scan_library`, `find_sidecars`, `match_audio_file` — filesystem scanning
- `parse_sidecar` — filesystem I/O (reads sidecar from disk)
- `extract_embedded_lyrics` — replaced by unified `fetch_lyrics()` API method
- `_resolve_priority` — no competing sources
- `_validate_library_paths`, `_is_under_roots`, `_path_parts` — path helpers for filesystem-based scoping
- `_normalize_path` — items keyed by ID, not path

**Retained:**
- `strip_lrc_tags` — defensive text normalization (Emby subtitle endpoint may return raw LRC content with timestamps; Jellyfin returns structured JSON but `strip_lrc_tags` is still called defensively)

**Config/CLI:**
- `library_paths` field on Config
- `--embedded-lyrics` / `--no-embedded-lyrics` / `--lyrics-priority` flags
- `--clear` flag (subsumed by `--overwrite` behavior on `rate` + new `reset` subcommand)
- `TAGLRC_LIBRARY_PATH` env var
- `EMBY_URL` / `JELLYFIN_URL` / `EMBY_API_KEY` / `JELLYFIN_API_KEY` env vars (replaced by named server model)
- `--server-type` flag (replaced by auto-detection)
- `--server-type both` mode (replaced by `--server NAME1 --server NAME2` or multi-select prompt)
- `genres` subcommand (folded into `configure`)

**DetectionResult fields:**
- `sidecar_path` → removed
- `audio_path` → renamed to `server_path: str | None` (contains server-reported path, e.g. `/share/music/...`)
- `source_conflict` → removed
- `source` simplified to `"lyrics"` | `"genre"` | `"force"` | `"reset"`

**CSV report columns:**
- `sidecar` column → removed
- `source_conflict` column → removed
- `track` column derived from server item `Path` filename (no filesystem fallback needed)
- `artist`/`album` from server metadata exclusively (no `_path_parts` filesystem fallback)

### What Stays Unchanged

- `classify_lyrics` — word detection engine
- `strip_lrc_tags` — text normalization (retained as defensive step)
- `_decide_rating_action` — rating logic (refactored to handle overwrite/skip behavior)
- `_decide_clear_action` — subsumed by `reset` subcommand and `--overwrite` behavior on `rate`
- `MediaServerClient` (extended, not replaced)
- Word lists, false positives, TOML detection config
- `--dry-run`, `--report`, `-v/--verbose`

---

## Subcommands

### `rate` (main workflow, replaces current `scan`)

Fetch lyrics via API, classify, set ratings. Genre fallback for tracks with no lyrics.

```
rate [options]
  --server NAME         Target a named server (repeatable; prompts if multiple configured and none specified)
  --library NAME        Scope to a specific library (default: all music libraries)
  --location NAME       Scope to a location within a library (e.g. "classical")
  -n, --dry-run         Analyze only, no server updates
  --report PATH         CSV report output path
  --overwrite           Re-evaluate and update tracks that already have a rating (default if not set in config)
  --skip-existing       Skip tracks that already have any rating
```

**Overwrite behavior:** By default, `rate` overwrites existing ratings when the evaluation result differs (including clearing a rating if lyrics are now clean). `--skip-existing` skips tracks that already have any rating. The default can be changed via `configure` (preference: overwrite vs skip). CLI flags override the configured default per-run.

### `force` (fixed rating, no evaluation — replaces current `rate`)

Set a fixed rating on all tracks in scope. No lyrics evaluation. Overwrites existing ratings by default.

```
force RATING [options]
  --server NAME         Target a named server (repeatable)
  --library NAME        Scope to a specific library
  --location NAME       Scope to a location within a library
  -n, --dry-run         Analyze only, no server updates
  --report PATH         CSV report output path
  --skip-existing       Skip tracks that already have any rating
```

### `reset` (remove all ratings in scope)

Remove `OfficialRating` from all tracks in scope. Destructive operation.

```
reset [options]
  --server NAME         Target a named server (repeatable)
  --library NAME        Scope to a specific library
  --location NAME       Scope to a location within a library
  -n, --dry-run         Analyze only, no server updates
  --report PATH         CSV report output path
```

### `configure` (interactive wizard)

Interactive setup that writes TOML config and `.env` file. Subsumes the current `genres` subcommand.

```
configure [options]
```

**Wizard flow:**
1. Prompt for server URL
2. Auto-detect server type via `/System/Info/Public`
3. Prompt for server label (used for TOML section name and env var prefix)
4. Prompt for username/password → authenticate via `POST /Users/AuthenticateByName` → save token to `.env` as `{LABEL}_API_KEY`. Retry on auth failure. Note: LDAP/SSO backends may not support this endpoint — fall back to prompting for a manually created API key.
5. Discover music libraries and locations
6. Assess genres — recommend instrumental/clean genres for G-rating, explain why genre-based rating complements lyrics detection ("these genres are typically instrumental — rating them G catches tracks that have no lyrics to evaluate at all")
7. Walk through detection rule customization (word lists, false positives)
8. Configure preferences (re-rate behavior: skip or overwrite)
9. Write TOML and `.env`

**Behavior based on state:**
- No TOML/ENV exists → full wizard
- TOML/ENV exists, no args → TUI showing current config, editable
- Args supplied → targeted updates

**Auth details:**
- `X-Emby-Authorization: MediaBrowser Client="SetMusicParentalRating", Device="{hostname}", DeviceId="{unique}", Version="{version}"` — shows up in server dashboard as identifiable device
- Password is used once to obtain token, never stored

**Shared options** (all subcommands): `--config`, `--env-file`, `-v/--verbose`

**CLI overrides** (for zero-config one-off use): `--server-url`, `--api-key`

---

## Named Server Model

Replaces the current per-platform env var scheme.

**TOML:**
```toml
[servers.home-emby]
url = "http://192.168.1.126:8096"
# type auto-detected on connect

[servers.home-jellyfin]
url = "http://192.168.1.126:8097"
```

**.env:**
```bash
HOME_EMBY_API_KEY=token-from-configure
HOME_JELLYFIN_API_KEY=token-from-configure
```

Convention: `{LABEL_UPPER}_API_KEY` where label comes from TOML section name (hyphens → underscores).

**Server type auto-detection** via `GET /System/Info/Public` (unauthenticated):
- Primary: `ProductName == "Jellyfin Server"` → Jellyfin; absent or null → Emby
- Fallback: `Server` response header — `Kestrel` → Jellyfin, `UPnP/1.0 DLNADOC/1.50` → Emby
- Error handling: if `/System/Info/Public` is unreachable, prompt user to specify type manually

**Multi-server behavior:**
- One server configured → use it automatically
- Multiple servers, none specified → prompt user to select (multiple choice, including "All")
- `--server NAME` → explicit selection (repeatable: `--server home-emby --server home-jellyfin`)
- `--server-url` + `--api-key` → one-off override, no TOML needed

---

## Lyrics Fetching

### Emby

**Discovery:** `MediaStreams` in bulk prefetch response contains entries with `Type=Subtitle, Codec=lrc, IsExternal=true` for sidecar lyrics. Embedded lyrics appear in `Extradata` on internal subtitle streams (`IsExternal=false, Type=Subtitle`).

**Precedence:** Try sidecar first (external subtitle stream), fall back to embedded (`Extradata`). Stop at first non-empty result.

**Fetch:** `GET /Videos/{itemId}/{mediaSourceId}/Subtitles/{streamIndex}/Stream.txt` — returns lyrics text. Apply `strip_lrc_tags` defensively (endpoint may return raw LRC content with timestamps on some Emby versions).

**Note:** The `/Videos/` endpoint path is used even for Audio items — verified on Emby 4.9.x. This should be re-verified during implementation (Issue #1).

### Jellyfin

**Fetch:** `GET /Audio/{itemId}/Lyrics` — returns JSON with `Lyrics[]` array of `{Text, Start}` objects. Extract `Text` fields and join with newlines.

### Unified Interface

`MediaServerClient.fetch_lyrics(item) -> str | None` — abstracts the server-specific logic. Returns plain text lyrics (normalized via `strip_lrc_tags`) or `None` if the track has no lyrics.

---

## Library/Location Scoping

**Discovery:** `GET /Library/VirtualFolders` returns libraries with `Name`, `CollectionType`, and `Locations[]`.

**Scoping mechanism:** Filter the bulk prefetch query using the library's internal ID or parent folder IDs from the VirtualFolders response, rather than path-based matching. This avoids reintroducing path-comparison logic.

- Default: process all libraries where `CollectionType == "music"`
- `--library Music` → filter to that library name
- `--location classical` → match against location paths (e.g. `/classical/` matches "classical"), use the location's folder ID to scope the query
- `rate` uses scoping to filter which items are evaluated
- `force` uses scoping to filter which items receive the fixed rating
- `reset` uses scoping to filter which items have ratings removed

---

## Issue Breakdown

### Milestone: API-Driven Lyrics (Layer 1)

| # | Issue | Labels | Dependencies |
|---|---|---|---|
| 1 | Add Emby lyrics fetch via subtitle stream endpoint | `enhancement` | — |
| 2 | Unify lyrics fetch: single `fetch_lyrics()` method | `enhancement` | #1 |
| 3 | Replace filesystem scan with API-driven lyrics pass | `enhancement` | #2 |
| 4 | Remove `--embedded-lyrics`, `--lyrics-priority`, and source conflict tracking | `enhancement` | #3 |
| 5 | Remove filesystem-only code (`scan_library`, `find_sidecars`, `parse_sidecar`, etc.) | `enhancement` | #3 |

### Milestone: Server & Scoping (Layer 2)

| # | Issue | Labels | Dependencies |
|---|---|---|---|
| 6 | Auto-detect server type via `/System/Info/Public` | `enhancement` | — |
| 7 | Named server model (TOML `[servers.*]` + `{LABEL}_API_KEY` env vars) | `enhancement` | #6 |
| 8 | Remove `--server-type` flag and old env var scheme | `enhancement` | #7 |
| 9 | Add library/location discovery and `--library`/`--location` scoping | `enhancement` | #7 |
| 10 | Rename subcommands: `scan`→`rate`, `rate`→`force`; add `reset`; fold `genres` into `configure` | `enhancement` | #9 |
| 11 | Add `--overwrite`/`--skip-existing` behavior to `rate` and `force` | `enhancement` | #10 |

### Milestone: Configure Wizard (Layer 3)

| # | Issue | Labels | Dependencies |
|---|---|---|---|
| 12 | Add `configure` subcommand — server connection and auth | `enhancement` | #7 |
| 13 | Add `configure` — library & genre discovery with recommendations | `enhancement` | #12 |
| 14 | Add `configure` — detection rules and preferences (including overwrite/skip default) | `enhancement` | #12 |
| 15 | Add `configure` — TUI for existing config (view/edit mode) | `enhancement` | #14 |

### Milestone: Cleanup

| # | Issue | Labels | Dependencies |
|---|---|---|---|
| 16 | Dead code analysis and removal | `enhancement` | Layer 1-3 complete |
| 17 | Documentation overhaul (CLAUDE.md, README.md, TOML example) | `documentation` | #16 |
