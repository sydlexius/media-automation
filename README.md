# Emby Scripts

Standalone utilities for managing an [Emby](https://emby.media/) media server.

## TagExplicitLyrics.py

Scans sidecar lyric files (`.lrc`, `.txt`) for explicit content and sets `OfficialRating` on matching audio tracks via the Emby API.

### How It Works

1. Recursively finds `.lrc` and `.txt` sidecar files under a music library path
2. Strips LRC timestamps/metadata to extract plain lyric text
3. Runs tiered word detection against configurable word lists:
   - **R** — strong profanity (stem matching: `fuck`, `shit`, etc.)
   - **PG-13** — moderate profanity (stem matching: `bitch`, `whore`, etc.)
4. Matches each sidecar to its audio file by filename stem
5. Looks up the audio file in Emby via a bulk prefetch of all Audio items
6. Sets `OfficialRating` on the Emby item via a GET-then-POST round-trip

Tracks without sidecar files are never touched. No assumptions are made about content based on sidecar presence or absence alone.

### Requirements

- Python 3.11+ (uses `tomllib` from stdlib)
  - Python 3.8+ works if TOML config is not needed, or with `pip install tomli`
- No other external dependencies (pure stdlib)

### Quick Start

```bash
# 1. Copy the example env file and add your API key
cp .env.example .env
# edit .env → set EMBY_API_KEY and EMBY_URL

# 2. Dry run — analyze without touching Emby
python3 TagExplicitLyrics.py /path/to/music --dry-run --report report.csv

# 3. Live run — set ratings
python3 TagExplicitLyrics.py /path/to/music --report report.csv

# 4. Force-rate a known-clean library (e.g., classical)
python3 TagExplicitLyrics.py /path/to/classical --force-rating G

# 5. Clear stale ratings after fixing sidecar typos
python3 TagExplicitLyrics.py /path/to/music --clear
```

### CLI Reference

```
TagExplicitLyrics.py [library_path] [options]

Positional:
  library_path              Library root (overrides config)

Options:
  --config PATH             TOML config file (default: explicit_config.toml in script dir)
  --env-file PATH           .env file to load (default: .env in script dir; e.g. .env.prod)
  --emby-url URL            Emby server URL
  --emby-api-key KEY        Emby API key
  -n, --dry-run             Analyze only, no Emby updates
  -v, --verbose             Debug logging
  --report PATH             CSV report output path
  --clear                   Clear ratings from tracks whose sidecars are now clean
  --force-rating RATING     Skip detection; set this rating on ALL tracks in the path
```

### Configuration

Settings are merged in priority order: **CLI flags > env vars > `.env` file > TOML config > hardcoded defaults**.

**`.env`** — secrets only (one per environment):
```bash
# .env — local dev
EMBY_API_KEY=your-key-here
EMBY_URL=http://localhost:8096

# Use --env-file .env.prod to load a different env file
# Exported EMBY_URL / EMBY_API_KEY still take precedence
```

**`explicit_config.toml`** — word lists, library path, report output. Copy `explicit_config.example.toml` to get started. The script works without any config file using sensible defaults.

### Detection Details

**Stem matching** checks if a stem (e.g., `fuck`) appears as a substring of any word token. This catches conjugations and compounds (`fucking`, `motherfucker`). A bidirectional false-positive filter prevents words like `cocktail`, `circumstance`, and `cucumber` from triggering.

**Exact matching** uses word-boundary regex for terms that would cause too many false positives as stems (e.g., `hoe`, `piss`).

R-tier matches take priority over PG-13. If any R word is found, the track is rated R regardless of PG-13 matches.

### CSV Report

The `--report` flag produces a CSV with columns useful for admin review:

| Column | Description |
|--------|-------------|
| `artist` | From Emby metadata (`AlbumArtist`), falls back to directory structure |
| `album` | From Emby metadata (`Album`), falls back to directory structure |
| `track` | Audio filename |
| `sidecar` | Sidecar filename |
| `tier` | `R`, `PG-13`, or empty (clean) |
| `matched_words` | Semicolon-separated list of words that triggered detection |
| `previous_rating` | What `OfficialRating` was before this run |
| `action` | `set`, `cleared`, `already_correct`, `skipped`, `dry_run`, `error` |

This lets an admin spot false positives caused by lyric transcription errors (e.g., "cuming" instead of "coming") and take corrective action on the sidecar files.

### Emby API Notes

- Auth: `X-Emby-Token` header on every request
- Item listing: `GET /Items?Recursive=true&IncludeItemTypes=Audio&Fields=Path,OfficialRating,AlbumArtist,Album` (paginated)
- Item fetch: `GET /Users/{userId}/Items/{itemId}` (user-scoped; `GET /Items/{id}` returns 404)
- Item update: `POST /Items/{itemId}` with the full item body (GET-then-POST round-trip preserves existing metadata)
