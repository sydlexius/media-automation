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
python3 -m pytest tests/ -v --tb=short    # run all tests (if tests/ exists)
python3 -c "import SetMusicParentalRating"  # verify imports (CI smoke test)
```

### Pre-commit
```bash
pre-commit run --all-files
```

## Architecture

`SetMusicParentalRating.py` is a single-file script with no external dependencies — pure stdlib Python 3.11+.

### Key data flow
1. **Config merge** (`build_config`): CLI flags > `os.environ` > `.env` file > `explicit_config.toml` > hardcoded defaults
2. **Filesystem scan** (`scan_library`): finds sidecar files, matches each to an audio file by filename stem
3. **LRC parsing** (`strip_lrc_tags`, `parse_sidecar`): strips timestamps/metadata to get plain text
4. **Detection** (`classify_lyrics`): two-tier word detection — stem matching (substring with false-positive filter) and exact matching (word-boundary regex). R tier takes priority over PG-13.
5. **Server sync** (`process_library`): bulk prefetches all Audio items by path, then GET-then-POST round-trip per item to update `OfficialRating`
6. **Genre pass** (`process_library`, after sidecar loop): items matching `[detection.g_genres]` and without a sidecar receive a `G` rating

### API pattern (Emby and Jellyfin)
- Auth: `X-Emby-Token` header (Emby) or `X-MediaBrowser-Token` header (Jellyfin)
- Item reads are user-scoped: `GET /Users/{userId}/Items/{id}` (not `GET /Items/{id}` which returns 404)
- Item updates require the full item body: `POST /Items/{id}` with the complete JSON from the GET

### Configuration
- Secrets (API key, URL) go in `.env` at the repo root — never in TOML or committed files
- Use `--env-file .env.prod` to target the production server
- Use `--server-type jellyfin` (or `SERVER_TYPE=jellyfin` in `.env`) to target Jellyfin
- Word lists, library path, and genre allow-list go in `explicit_config.toml` (copy from `explicit_config.example.toml`)
- Only `explicit_config.example.toml` is committed; all other TOML variants are gitignored

## CI

GitHub Actions runs on push/PR to `main`:
- **Lint**: `ruff check .` and `ruff format --check .` (pinned ruff v0.15.5, Python 3.13)
- **Test**: import verification (`working-directory: SetMusicParentalRating`) + pytest (if `SetMusicParentalRating/tests/` exists)

Pre-commit hooks: `check-ast`, `check-yaml`, `end-of-file-fixer`, `trailing-whitespace`, ruff check+format.
