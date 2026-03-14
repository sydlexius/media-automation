# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Standalone Python utilities for managing an Emby or Jellyfin media server. Each script lives in its own subdirectory with its own README and TOML config. Shared server credentials (`.env`, `.env.prod`) live in the repo root.

Currently contains one script: `SetMusicParentalRating/SetMusicParentalRating.py` — scans sidecar lyric files (.lrc, .txt) for explicit content using tiered word detection (R / PG-13) and sets `OfficialRating` on matching audio tracks via the Emby or Jellyfin API.

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

# Dry run (analysis only, no server writes) — Emby (default)
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/music --dry-run --report report.csv

# Multiple library paths in a single run
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/music /path/to/classical --dry-run --report report.csv

# Dry run against Jellyfin
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/music --server-type jellyfin --dry-run

# Live run
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/music --report report.csv

# Production server
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/music --env-file .env.prod --config SetMusicParentalRating/explicit_config.prod.toml
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
1. **Config merge** (`build_config`): CLI flags > `os.environ` > `.env` file > `explicit_config.toml` > hardcoded defaults. `Config.library_paths` is a `list[Path]` — the CLI accepts multiple positional args, the TOML key `library_path` accepts both a string and an array, and the `TAGLRC_LIBRARY_PATH` env var provides a single path. Server-type resolution is two-phase: explicit `SERVER_TYPE` override wins; otherwise auto-detected from which `EMBY_URL`/`JELLYFIN_URL` vars are present (errors if both are set). `Config.__post_init__` precompiles exact-match regexes — any new config field that needs preprocessing belongs there.
2. **Filesystem scan** (`scan_library`): finds sidecar files, matches each to an audio file by filename stem. When multiple library paths are given, results are merged with deduplication.
3. **LRC parsing** (`strip_lrc_tags`, `parse_sidecar`): strips timestamps/metadata to get plain text. `extract_embedded_lyrics` does the same for `MediaSources.MediaStreams[].Extradata` from server items.
4. **Detection** (`classify_lyrics`): two-tier word detection — stem matching (substring with false-positive filter) and exact matching (word-boundary regex). R tier takes priority over PG-13.
5. **Server sync** (`process_library`): bulk prefetches all Audio items by path into `server_items: dict[str, dict]`, then runs three sequential passes, each skipping paths already in `handled_paths`. `_validate_library_paths` checks each path is absolute, exists, and is a directory. `_is_under_roots` checks whether a normalized path falls under any of the library roots (used by embedded and genre passes to scope server items). `_item_fields` extracts `(item_id, previous_rating, artist, album)` from each server item. `_decide_rating_action` / `_decide_clear_action` encapsulate the shared set/clear decision logic used across all passes.
   - **Sidecar pass**: processes all `(sidecar, audio)` pairs from `scan_library`; adds each audio path to `handled_paths`. When `config.embedded_lyrics` is on, each sidecar-pass item also checks the server item for embedded lyrics and calls `_resolve_priority` to pick the winning source — see `lyrics_priority` below.
   - **Embedded pass** (only when `config.embedded_lyrics`): for Emby, reads `Extradata` from prefetched `MediaSources`; for Jellyfin, calls `GET /Audio/{itemId}/Lyrics` per track. Adds matched paths to `handled_paths`
   - **Genre pass** (only when `config.g_genres`): assigns `G` to items not yet in `handled_paths` whose `Genres` list overlaps the allow-list
6. Each track produces one `DetectionResult`. `source` identifies which lyrics source determined the final rating: `"sidecar"` | `"embedded"` | `"genre"` | `"force"`. `source_conflict` is non-empty when both sidecar and embedded lyrics existed and disagreed (format: `"{loser}:{tier}->{WINNER}:{tier}"`). `action` records what happened: `set | cleared | skipped | already_correct | not_found_in_server | server_unavailable | error | no_audio_file | dry_run | dry_run_clear | g_genre | g_genre_already_correct | dry_run_g_genre`.

### API pattern (Emby and Jellyfin)
- Auth: `X-Emby-Token` header (Emby) or `X-MediaBrowser-Token` header (Jellyfin)
- Bulk audio listing: `GET /Users/{userId}/Items?Recursive=true&IncludeItemTypes=Audio&Fields=Path,OfficialRating,AlbumArtist,Album,Genres` (paginated at 500; `MediaSources` appended to `Fields` when `--embedded-lyrics` is enabled)
- Item reads are user-scoped: `GET /Users/{userId}/Items/{id}` (not `GET /Items/{id}` which returns 404)
- Item updates require the full item body: `POST /Items/{id}` with the complete JSON from the GET
- Genre listing: `GET /MusicGenres?Recursive=true` (used by `--list-genres`)

### Thread safety
The script is single-threaded. `process_library` uses shared mutable state
(`handled_paths: set`, `results: list`) that is not thread-safe. If parallelism
is ever added, these would need locking or per-thread accumulation.

### Configuration
- Secrets (API key, URL) go in `.env` at the repo root — never in TOML or committed files
- Use `--env-file .env.prod` to target the production server
- Use `--server-type jellyfin` (or `SERVER_TYPE=jellyfin` in `.env`) to target Jellyfin
- Word lists, library path, and genre allow-list go in `explicit_config.toml` (copy from `explicit_config.example.toml`)
- Only `explicit_config.example.toml` is committed; all other TOML variants are gitignored
- `lyrics_priority` (`"sidecar"` | `"embedded"` | `"most_explicit"`, default `"sidecar"`): when `embedded_lyrics` is on and a track has both a sidecar and embedded lyrics, controls which source wins. `most_explicit` picks whichever detected the higher tier. Applies to both Emby and Jellyfin.

## CI

GitHub Actions runs on push/PR to `main`:
- **Lint**: `ruff check .` and `ruff format --check .` (pinned ruff v0.15.5, Python 3.13)
- **Test**: import verification (`working-directory: SetMusicParentalRating`) + pytest (if `SetMusicParentalRating/tests/` exists)

Pre-commit hooks: `check-ast`, `check-yaml`, `end-of-file-fixer`, `trailing-whitespace`, ruff check+format.
