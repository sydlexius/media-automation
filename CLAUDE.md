# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Standalone Python utilities for managing an Emby or Jellyfin media server. Each script lives in its own subdirectory with its own README and TOML config. Shared server credentials (`.env`, `.env.prod`) live in the repo root.

Currently contains one script: `SetMusicParentalRating/SetMusicParentalRating.py` — fetches lyrics from the Emby or Jellyfin API, detects explicit content using tiered word detection (R / PG-13), and sets `OfficialRating` on matching audio tracks.

## Repository Layout

```text
/
├── .env / .env.prod          # shared credentials (gitignored)
├── .env.example              # credentials template (committed)
├── README.md                 # repo index
└── SetMusicParentalRating/
    ├── SetMusicParentalRating.py
    ├── README.md
    ├── explicit_config.example.toml   # committed template
    ├── explicit_config.toml           # gitignored (copy of example)
    ├── explicit_config.uat.toml       # gitignored
    ├── explicit_config.prod.toml      # gitignored
    └── tests/                         # not yet present; CI skips if absent
```

## Commands

### Run the script
```bash
# Run from the repo root:

# Dry run — evaluate lyrics, report but don't update server
python3 SetMusicParentalRating/SetMusicParentalRating.py rate --library Music --dry-run --report report.csv

# Scope to a specific library or location
python3 SetMusicParentalRating/SetMusicParentalRating.py rate --library Music --dry-run
python3 SetMusicParentalRating/SetMusicParentalRating.py rate --location classical --dry-run

# Target a specific named server
python3 SetMusicParentalRating/SetMusicParentalRating.py rate --server home-jellyfin --library Music --dry-run

# Force-rate a library as G (no lyrics evaluation)
python3 SetMusicParentalRating/SetMusicParentalRating.py force G --library "Classical Music"

# Remove all ratings from a library
python3 SetMusicParentalRating/SetMusicParentalRating.py reset --library Music --dry-run

# Skip tracks that already have a rating
python3 SetMusicParentalRating/SetMusicParentalRating.py rate --library Music --skip-existing --dry-run

# One-off server (no TOML config needed)
python3 SetMusicParentalRating/SetMusicParentalRating.py rate --server-url http://localhost:8096 --api-key YOUR_KEY --library Music --dry-run

# Production server
python3 SetMusicParentalRating/SetMusicParentalRating.py rate --env-file .env.prod --config SetMusicParentalRating/explicit_config.prod.toml --dry-run
```

### Lint & Format
```bash
ruff check .          # lint (run from repo root)
ruff check --fix .
ruff format .
ruff format --check . # CI uses this
```

### Tests
```bash
cd SetMusicParentalRating
python3 -m pytest tests/ -v --tb=short              # run all tests (if tests/ exists)
python3 -m pytest tests/test_foo.py::test_name -v   # run a single test
python3 -c "import SetMusicParentalRating"           # verify imports (CI smoke test)
```

### Pre-commit
```bash
pre-commit run --all-files
```

## Architecture

`SetMusicParentalRating.py` is a single-file script with no external dependencies — pure stdlib Python 3.11+.

### Key data flow
1. **CLI** (`build_parser`): three subcommands — `rate`, `force`, `reset` — with a shared parent parser for common options (`--config`, `--env-file`, `--server`, `--server-url`, `--api-key`, `--library`, `--location`, `-v`). **Config merge** (`build_config`): CLI flags > `os.environ` > `.env` file > `explicit_config.toml` > hardcoded defaults. **Server resolution** (`_resolve_servers`): `--server-url`+`--api-key` (one-off) > TOML `[servers.*]` sections. Server type (Emby/Jellyfin) is auto-detected via `GET /System/Info/Public`. `Config.__post_init__` precompiles exact-match regexes.
2. **Library scoping** (`_resolve_library_scope`): `GET /Library/VirtualFolders` discovers music libraries. `--library NAME` scopes by `ParentId`. `--location NAME` adds post-prefetch path filtering via `_filter_by_location`. Without flags, processes all audio items.
3. **Lyrics fetch** (`fetch_lyrics`): Emby: tries external subtitle streams (`GET /Videos/{id}/{msid}/Subtitles/{idx}/Stream.txt`), falls back to embedded `Extradata`. Jellyfin: `GET /Audio/{id}/Lyrics`. Returns plain text (LRC-stripped) or `None`.
4. **Detection** (`classify_lyrics`): two-tier word detection — stem matching (substring with false-positive filter) and exact matching (word-boundary regex). R tier takes priority over PG-13.
5. **Rating** (`process_library`): single-pass over prefetched items. For each: fetch lyrics → classify → set/skip rating. Items without lyrics fall through to genre allow-list check (`match_g_genre`). `--overwrite` (default) re-evaluates all tracks and clears ratings from clean tracks. `--skip-existing` skips tracks with any rating.
6. **Force rating** (`force_rate_library`): sets a fixed rating on all tracks in scope without lyrics evaluation. Respects `--overwrite`/`--skip-existing`.
7. **Reset** (`reset_library`): removes `OfficialRating` from all tracks in scope.
8. Each track produces one `DetectionResult`. `source`: `"lyrics"` | `"genre"` | `"force"` | `"reset"`. `action`: `set | cleared | skipped | already_correct | not_found_in_server | error | dry_run | dry_run_clear | g_genre | g_genre_already_correct | dry_run_g_genre`.

### API pattern (Emby and Jellyfin)
- Auth: `X-Emby-Token` header (Emby) or `X-MediaBrowser-Token` header (Jellyfin)
- Server type detection: `GET /System/Info/Public` (unauthenticated) — `ProductName == "Jellyfin Server"` → Jellyfin; fallback: `Server` header
- Library discovery: `GET /Library/VirtualFolders` — returns `Name`, `ItemId`, `CollectionType`, `Locations[]`
- Bulk audio listing: `GET /Users/{userId}/Items?Recursive=true&IncludeItemTypes=Audio&Fields=Path,OfficialRating,AlbumArtist,Album,Genres` (paginated at 500; `MediaSources` appended when lyrics fetch needs stream info; `&ParentId={id}` for library scoping)
- Item reads are user-scoped: `GET /Users/{userId}/Items/{id}` (not `GET /Items/{id}` which returns 404)
- Item updates require the full item body: `POST /Items/{id}` with the complete JSON from the GET
- Genre listing: `GET /MusicGenres?Recursive=true`

### Thread safety
The script is single-threaded. `process_library` uses shared mutable state
(`handled_paths: set`, `results: list`) that is not thread-safe. If parallelism
is ever added, these would need locking or per-thread accumulation.

### Named server model
Servers are defined in TOML `[servers.*]` sections with API keys in `.env` as `{LABEL}_API_KEY` (hyphens → underscores):
```toml
[servers.home-emby]
url = "http://192.168.1.126:8096"
```
```bash
HOME_EMBY_API_KEY=your-key
```
Server type is auto-detected; override with `type = "emby"` in the TOML section. `--server NAME` selects a specific server. `--server-url` + `--api-key` provides one-off credentials.

### Configuration
- API keys go in `.env` at the repo root as `{LABEL}_API_KEY` — never in TOML or committed files
- Use `--env-file .env.prod` to target production credentials
- Word lists, library path, and genre allow-list go in `explicit_config.toml` (copy from `explicit_config.example.toml`)
- Only `explicit_config.example.toml` is committed; all other TOML variants are gitignored
- `overwrite` (default `true`): when true, `rate` re-evaluates all tracks including clearing ratings from clean tracks; when false, skips tracks with existing ratings (`--skip-existing`)

## CI

GitHub Actions runs on push/PR to `main`:
- **Lint**: `ruff check .` and `ruff format --check .` (pinned ruff v0.15.5, Python 3.13)
- **Test**: import verification (`working-directory: SetMusicParentalRating`) + pytest (if `SetMusicParentalRating/tests/` exists)

Pre-commit hooks: `check-ast`, `check-yaml`, `end-of-file-fixer`, `trailing-whitespace`, ruff check+format.
