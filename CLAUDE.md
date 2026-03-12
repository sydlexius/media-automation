# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Standalone Python utilities for managing an Emby media server. Currently contains one script: `TagExplicitLyrics.py` — scans sidecar lyric files (.lrc, .txt) for explicit content using tiered word detection (R / PG-13) and sets `OfficialRating` on matching audio tracks via the Emby API.

## Commands

### Run the script
```bash
# Dry run (analysis only, no Emby writes)
python3 TagExplicitLyrics.py /path/to/music --dry-run --report report.csv

# Live run
python3 TagExplicitLyrics.py /path/to/music --report report.csv
```

### Lint & Format
```bash
ruff check .          # lint
ruff check --fix .    # lint with auto-fix
ruff format .         # format
ruff format --check . # format check (CI uses this)
```

### Tests
```bash
python -m pytest tests/ -v --tb=short    # run all tests (if tests/ exists)
python -c "import TagExplicitLyrics"     # verify imports (CI smoke test)
```

### Pre-commit
```bash
pre-commit run --all-files
```

## Architecture

The codebase is a single-file script (`TagExplicitLyrics.py`, ~870 lines) with no external dependencies — pure stdlib Python 3.11+.

### Key data flow
1. **Config merge** (`build_config`): CLI flags > `os.environ` > `.env` file > `explicit_config.toml` > hardcoded defaults
2. **Filesystem scan** (`scan_library`): finds sidecar files, matches each to an audio file by filename stem
3. **LRC parsing** (`strip_lrc_tags`, `parse_sidecar`): strips timestamps/metadata to get plain text
4. **Detection** (`classify_lyrics`): two-tier word detection — stem matching (substring with false-positive filter) and exact matching (word-boundary regex). R tier takes priority over PG-13.
5. **Emby sync** (`process_library`): bulk prefetches all Audio items by path, then GET-then-POST round-trip per item to update `OfficialRating`

### Emby API pattern
- Auth via `X-Emby-Token` header
- Item reads are user-scoped: `GET /Users/{userId}/Items/{id}` (not `GET /Items/{id}` which returns 404)
- Item updates require the full item body: `POST /Items/{id}` with the complete JSON from the GET

### Configuration
- Secrets (API key, URL) go in `.env` — never in TOML or committed files
- Use `--env-file .env.prod` to target a different server environment
- Word lists and library path go in `explicit_config.toml` (copy from `explicit_config.example.toml`)
- `.env.example` is the template for `.env`; `explicit_config.example.toml` is the template for TOML config

## CI

GitHub Actions runs on push/PR to `main`:
- **Lint**: `ruff check .` and `ruff format --check .` (pinned ruff v0.15.5, Python 3.13)
- **Test**: import verification + pytest (if `tests/` directory exists)

Pre-commit hooks: `check-ast`, `check-yaml`, `end-of-file-fixer`, `trailing-whitespace`, ruff check+format.
