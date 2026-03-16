# Rating Orchestration — Design Spec

**Date:** 2026-03-15
**Status:** Draft
**Scope:** Issues #79–#83 (Milestone: Rust Rewrite — Rating Orchestration)
**Depends on:** Detection Engine (#75–#78), Server API Client (#69–#74), Config (#67–#68) — all merged on main

---

## Problem

The Rust rewrite has config loading, server API client, and detection engine, but
no orchestration logic. The `rating.rs` and `report.rs` files are stubs. The three
subcommands (`rate`, `force`, `reset`) print "not yet implemented" and exit. This
milestone ports the Python orchestration logic (`process_library`,
`force_rate_library`, `reset_library`, `write_report`, multi-server loop, and
summary output) to Rust.

## Approach

**Approach B: Shared Scaffolding + Workflow Enum.** Extract the common orchestration
scaffolding (connect → scope → prefetch → iterate → collect results) into a shared
function. Per-item logic varies by workflow. A `RatingAction` enum replaces Python's
stringly-typed `action` field. A `Source` enum replaces the `source` string.

---

## Core Types (`rating.rs`)

### `RatingAction` enum

Represents the outcome of processing a single track:

| Variant | Meaning |
|---------|---------|
| `Set` | Rating was applied to server |
| `Cleared` | Rating was removed (set to empty) |
| `Skipped` | Skipped (has existing rating + skip-existing, or clean lyrics without rating) |
| `AlreadyCorrect` | Rating already matches desired value |
| `DryRun` | Would have set (dry-run mode) |
| `DryRunClear` | Would have cleared (dry-run mode) |
| `Error(String)` | Update failed (non-auth server error) |

### `Source` enum

Why a track was rated:

| Variant | Meaning |
|---------|---------|
| `Lyrics` | Rating determined by lyrics classification |
| `Genre` | Rating determined by genre allow-list (G) |
| `Force` | Force subcommand or config `force_rating` |
| `Reset` | Reset subcommand |

### `ItemResult` struct

One per track processed (replaces Python's `DetectionResult`):

| Field | Type | Description |
|-------|------|-------------|
| `item_id` | `String` | Server item ID |
| `path` | `Option<String>` | Server-reported file path |
| `artist` | `Option<String>` | Album artist from server metadata |
| `album` | `Option<String>` | Album name from server metadata |
| `tier` | `Option<String>` | "R", "PG-13", "G", or None (clean/reset) |
| `matched_words` | `Vec<String>` | Words that triggered the tier |
| `previous_rating` | `Option<String>` | Rating before processing |
| `action` | `RatingAction` | What happened |
| `source` | `Source` | Why |
| `server_name` | `String` | Config label (e.g. "home-emby") |

Differences from Python's `DetectionResult`:
- No `sidecar_path` (filesystem scanning removed)
- No `server_type` field (`server_name` is more useful for multi-server reports)
- `action` and `source` are enums, not strings
- `path` is the server-reported path (for track name extraction in reports)

---

## Shared Scaffolding

All three workflows share the same setup steps, extracted into a common function:

1. **Auto-detect server type** if not specified in TOML config
2. **Create API client** (`MediaServerClient`)
3. **Resolve library/location scope** — call `discover_libraries()`, match
   `--library`/`--location` to a parent ID and optional location path
4. **Prefetch all audio items** in scope (paginated bulk fetch, scoped by parent ID)
5. **Filter by location** if `--location` was used (post-prefetch path-prefix filtering)
6. **Iterate items** — call the workflow-specific per-item logic
7. **Collect results** into `Vec<ItemResult>`

### Library/location scoping

Ported from Python's `_resolve_library_scope` and `_filter_by_location`:

- `--library Music` → find the library by name (case-insensitive) in
  `discover_libraries()` response → use its `ItemId` as `parent_id` for prefetch
- `--location classical` → find the location by leaf directory name across all
  music libraries → use the parent library's `ItemId` for prefetch, then post-filter
  items whose path starts with the location's server-side path
- `--library Music --location classical` → find location within the specified library
- Neither flag → process all audio items across all music libraries
- Error if library/location not found (list available options in error message)

### Force rating lookup

During the `rate` workflow, before evaluating lyrics for each item, the scaffolding
checks `ServerConfig.libraries` for a matching `force_rating`:

1. If the current `--location` matches a location config with `force_rating`, use it
2. Else if the current `--library` matches a library config with `force_rating`, use it
3. Else proceed to normal lyrics evaluation

`--ignore-forced` suppresses all force_rating lookups and always evaluates lyrics.

The lookup uses the library/location names from CLI flags (or discovered names when
processing all libraries). Location-level `force_rating` takes precedence over
library-level.

---

## Workflows

### `rate` (issue #79)

For each audio track in scope:

1. **Check config force_rating** — if the track's library/location has a
   `force_rating` in TOML config, apply that rating directly (skip lyrics
   evaluation). Unless `--ignore-forced` was passed.
2. **Fetch lyrics** — call the server API. If auth error (401/403), abort
   everything. Other errors: log warning, treat as "no lyrics."
3. **Classify lyrics** — run through the detection engine. Gets a tier (R, PG-13,
   or clean) and matched words.
4. **If explicit (R or PG-13):**
   - Skip if `--skip-existing` and track already has a rating
   - Skip if rating already matches the tier
   - Otherwise set the rating (or log in dry-run)
5. **If clean lyrics + overwrite enabled + track has a rating:** Clear the rating
   (the track was re-evaluated and is now clean)
6. **If no lyrics at all:** Try genre fallback — if track's genre matches the
   allow-list, set G rating (same skip/overwrite logic)

### `force` (issue #80)

For each track in scope:
1. Set the CLI-provided rating (e.g., `smpr force G --library Music`)
2. Respects `--skip-existing` (won't overwrite existing ratings)
3. Skips tracks already at the target rating (`AlreadyCorrect`)
4. Dry-run support

No lyrics evaluation, no detection engine.

### `reset` (issue #81)

For each track in scope:
1. Remove the rating (set `OfficialRating` to empty string)
2. Skips tracks that have no rating
3. Dry-run support

---

## Rating Application

Shared helper function for applying a rating change to a single item (ported from
Python's `_apply_rating`):

1. `GET /Users/{uid}/Items/{id}` — fetch full item body
2. Set `OfficialRating` on the JSON value
3. `POST /Items/{id}` — send modified body back
4. Return `Set` on success, `Error(message)` on failure

For clearing: same flow but set `OfficialRating` to empty string, return `Cleared`.

### Error handling

- **Per-item failures** (lyrics fetch, rating update): log warning, record
  `Error(message)` in result, continue to next item
- **Auth errors (401/403):** abort the entire workflow immediately
- **Prefetch failure:** abort the workflow for this server, continue to next server

---

## CSV Report (issue #82)

### Columns

| Column | Content |
|--------|---------|
| `artist` | Album artist from server metadata |
| `album` | Album name from server metadata |
| `track` | Filename extracted from server-reported path |
| `tier` | R, PG-13, G, or empty |
| `matched_words` | Semicolon-separated trigger words |
| `previous_rating` | Rating before processing |
| `action` | set, cleared, skipped, already_correct, dry_run, dry_run_clear, error |
| `source` | lyrics, genre, force, reset |
| `server` | Config label name (e.g. "home-emby") |

### Behavior

- Creates parent directories if they don't exist
- Errors writing the report are logged but don't abort the run
- Combines results from all servers into a single report file
- No `sidecar` column (filesystem scanning removed in API-driven refactor)

---

## Multi-Server Loop and Summary (issue #83)

### Multi-server loop (`main.rs`)

1. Iterate over all servers in the resolved config
2. For each server: auto-detect type if needed → run the chosen workflow → collect
   results
3. If a server fails (connection error, prefetch failure), log the error and
   **continue to the next server** — don't abort the whole run
4. After all servers: write the combined report (if `--report` was given)
5. Exit with non-zero status if any server failed

### Summary output

Printed to stdout:
- After each server when multi-server
- Once at the end when single server

Counts displayed:

| Counter | Description |
|---------|-------------|
| Lyrics evaluated | Tracks where lyrics were fetched and classified |
| R-rated | Tracks classified as R |
| PG-13 | Tracks classified as PG-13 |
| Clean | Tracks with lyrics but no explicit content |
| Ratings set | Tracks where rating was applied |
| Already correct | Tracks where rating matched |
| Ratings cleared | Tracks where rating was removed |
| G (genre-matched) | Tracks rated G via genre allow-list |
| Already G (genre) | Genre-matched tracks already rated G |
| Dry-run would act | Tracks that would be changed in non-dry-run mode |
| Errors | Tracks where update failed |

Simplified from Python — removes sidecar-era counters.

---

## Module Structure

```
smpr/src/
├── main.rs          # Multi-server loop, summary output, CLI wiring
├── rating.rs        # Core types, shared scaffolding, three workflows, rating helpers
├── report.rs        # CSV report writer
├── config/          # (existing) Config loading
├── detection.rs     # (existing) Detection engine
├── server/          # (existing) Server API client
├── util.rs          # (existing) LRC tag stripping
└── tui/             # (existing) Configure wizard placeholder
```

`rating.rs` may be split into submodules if it grows large, but starts as a single
file since the three workflows are tightly coupled through shared types and helpers.

---

## Testing Strategy

### Unit tests (no network, run in CI)

- **Decision logic:** Given a track with specific current rating + tier +
  overwrite/dry-run settings → verify correct action
- **Library scoping:** Given mock library discovery responses → verify correct
  parent_id and location path resolution, including error cases
- **Location filtering:** Given items with various paths + a location path → verify
  correct filtering
- **Force rating lookup:** Given a server config with library/location
  force_ratings → verify correct lookup precedence (location > library > none)
- **Report writing:** Given a list of results → verify CSV output has correct
  columns and content
- **Summary counting:** Given result lists with various actions → verify correct
  counts

### Integration tests (UAT servers, gated behind `SMPR_UAT_TEST=1`)

- **Rate dry-run** against Jellyfin at localhost:8097 — verify items are fetched,
  lyrics classified, no mutations
- **Force dry-run** against Jellyfin — verify all items get target rating in results
- **Reset dry-run** against Jellyfin — verify items with ratings show dry_run_clear
- **Library scoping** — verify `--library Music` narrows the item set
- All integration tests are **read-only** (dry-run only) — no mutations to UAT data

---

## PR Breakdown

### PR 1: Core types + shared scaffolding + rate workflow (#79)

**Files:**
- `src/rating.rs` — `RatingAction`, `Source`, `ItemResult`, shared scaffolding
  (library scoping, location filtering, force_rating lookup), `rate` workflow,
  rating application helper
- `src/main.rs` — wire `rate` subcommand to the workflow (single server only;
  multi-server in PR 4)

Delivers the most complex workflow first. Self-contained and testable.

### PR 2: Force + reset workflows (#80 + #81)

**Files modified:**
- `src/rating.rs` — add `force` and `reset` workflow functions
- `src/main.rs` — wire `force` and `reset` subcommands

Depends on PR 1. Both workflows are simple and share the scaffolding from PR 1.

### PR 3: CSV report writer (#82)

**Files:**
- `src/report.rs` — `write_report()` function
- `src/main.rs` — wire report writing after workflow execution

Depends on PR 1 (needs `ItemResult` type). Independent of PR 2.

### PR 4: Multi-server loop + summary output (#83)

**Files modified:**
- `src/main.rs` — multi-server iteration, per-server error isolation, summary
  printing, combined report writing

Depends on PRs 1–3. Completes the milestone.

### Merge protocol

Each PR waits for CodeRabbit to finish reviewing before merging. Dismiss stale
`CHANGES_REQUESTED` reviews + `@coderabbitai approve` before merging.

---

## What's NOT in Scope

- Configure wizard / TUI (separate milestone)
- Report `output_path` template variables (future enhancement)
- Parallel item processing / async (single-threaded, matches Python)
- Location-level force_rating without `--library`/`--location` flags (force_rating
  only applies when the orchestration knows which library/location context the item
  belongs to — this matches Python behavior)

---

## References

- Python source: `SetMusicParentalRating/SetMusicParentalRating.py` — `process_library` (line 1226), `force_rate_library` (line 1398), `reset_library` (line 1486), `_decide_rating_action` (line 1592), `write_report` (line 1042), `main` (line 1954)
- API-driven refactor design: `docs/superpowers/specs/2026-03-13-api-driven-refactor-design.md`
- Config spec: `docs/superpowers/specs/2026-03-14-config-and-cli-design.md`
- Server API client spec: `docs/superpowers/specs/2026-03-14-server-api-client-design.md`
