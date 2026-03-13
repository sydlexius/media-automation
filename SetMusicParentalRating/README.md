# SetMusicParentalRating

Standalone utility for managing an [Emby](https://emby.media/) or [Jellyfin](https://jellyfin.org/) media server.

## SetMusicParentalRating.py

Scans sidecar lyric files (`.lrc`, `.txt`) for explicit content and sets `OfficialRating` on matching audio tracks via the Emby or Jellyfin API.

### How It Works

1. Recursively finds `.lrc` and `.txt` sidecar files under a music library path
2. Matches each sidecar to its audio file by filename stem
3. Strips LRC timestamps/metadata to extract plain lyric text
4. Runs tiered word detection against configurable word lists:
   - **R** — strong profanity (stem matching against a configurable word list)
   - **PG-13** — moderate profanity (stem matching against a configurable word list)
5. Looks up the audio file in the media server via a bulk prefetch of all Audio items
6. Sets `OfficialRating` on the item via a GET-then-POST round-trip
7. *(Optional)* Embedded-lyrics pass: tracks without a sidecar are checked for lyrics embedded in their audio metadata (ID3 `USLT`, Vorbis `LYRICS`, etc.) when `--embedded-lyrics` is enabled
8. *(Optional)* Genre pass: any audio item whose `Genres` field contains an entry from `[detection.g_genres]` and has not been handled by the sidecar or embedded pass receives a `G` rating

**Priority rule**: sidecar → embedded → genre. Any track processed by the sidecar or embedded pass (explicit or clean) is excluded from the genre pass entirely.

### Requirements

- Python 3.11+ (uses `tomllib` from stdlib)
  - Python 3.8+ works if TOML config is not needed, or with `pip install tomli`
- No other external dependencies (pure stdlib)

### Quick Start

```bash
# Run from the repo root:

# 1. Copy the example env file and add your API key(s)
cp .env.example .env
# edit .env → set EMBY_API_KEY and EMBY_URL (for Emby)
#           or JELLYFIN_API_KEY and JELLYFIN_URL (for Jellyfin)

# 2. Dry run — analyze without touching the server (default)
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/music --dry-run --report report.csv

# 3. Dry run against Jellyfin
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/music --server-type jellyfin --dry-run

# 4. Live run — set ratings
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/music --report report.csv

# 5. Force-rate a known-clean library (e.g., classical)
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/classical --force-rating G

# 6. Clear stale ratings after fixing sidecar typos
python3 SetMusicParentalRating/SetMusicParentalRating.py /path/to/music --clear

# 7. Discover what genre strings exist in your library (for g_genres config)
python3 SetMusicParentalRating/SetMusicParentalRating.py --list-genres
python3 SetMusicParentalRating/SetMusicParentalRating.py --server-type jellyfin --list-genres
```

### CLI Reference

```text
SetMusicParentalRating.py [library_path] [options]

Positional:
  library_path              Library root (overrides config)

Options:
  --config PATH             TOML config file (default: explicit_config.toml in script dir)
  --env-file PATH           .env file to load (default: .env in repo root; e.g. --env-file .env.prod)
  --server-type TYPE        'emby' or 'jellyfin' — auto-detected from configured server URLs
                            (env vars or TOML) when only one server is active; can also be
                            set via SERVER_TYPE env var or [general].server_type in TOML
  --server-url URL          Server URL — overrides the env var for the active server type
  --api-key KEY             API key — overrides the env var for the active server type
  -n, --dry-run             Analyze only, no server updates
  -v, --verbose             Debug logging
  --report PATH             CSV report output path
  --clear                   Clear ratings from tracks whose sidecars are now clean
  --force-rating RATING     Skip detection; set this rating on ALL tracks in the path
  --list-genres             Print all Audio genre tags from the server, then exit
                            (useful for building [detection.g_genres] in the config;
                            library_path is not required)
  --embedded-lyrics         Scan embedded lyrics tags for explicit content. On Emby,
                            adds MediaSources to the bulk prefetch. On Jellyfin, adds
                            one GET /Audio/{id}/Lyrics per track in scope (including
                            sidecar-matched tracks, for --lyrics-priority resolution).
                            (default: off)
  --no-embedded-lyrics      Explicitly disable embedded-lyrics scanning, overriding
                            detection.embedded_lyrics = true in the TOML config
  --lyrics-priority {sidecar,embedded,most_explicit}
                            Which source wins when a track has both a sidecar (.lrc/.txt)
                            and embedded lyrics. Only applies when --embedded-lyrics is on.
                            Default: sidecar. most_explicit picks whichever detected the
                            higher tier.
```

### Configuration

Settings are merged in priority order: **CLI flags > env vars > `.env` file > TOML config > hardcoded defaults**.

**`.env`** — secrets only (one per environment):
```bash
# Single-server .env — type is auto-detected from which vars are set
EMBY_URL=http://localhost:8096
EMBY_API_KEY=your-emby-key-here

# Both servers configured — SERVER_TYPE is then required to disambiguate
EMBY_URL=http://localhost:8096
EMBY_API_KEY=your-emby-key-here
JELLYFIN_URL=http://localhost:8097
JELLYFIN_API_KEY=your-jellyfin-key-here
SERVER_TYPE=emby   # or override per-run with --server-type

# Use --env-file .env.prod to load a different env file
# Exported env vars still take precedence over .env
```

**`explicit_config.toml`** — word lists, library path, report output, and genre allow-list. Copy `explicit_config.example.toml` to get started. The script works without any config file using sensible defaults.

**`[detection]`** — top-level detection settings:

```toml
[detection]
embedded_lyrics = false   # set to true to scan embedded tag lyrics for tracks with no sidecar
```

On Emby, enabling `embedded_lyrics` adds `MediaSources` to the bulk prefetch, increasing payload size on large libraries. On Jellyfin, it adds per-track `GET /Audio/{itemId}/Lyrics` calls instead (see Jellyfin note below).

> **Jellyfin note:** When `--server-type jellyfin` is used, `GET /Audio/{itemId}/Lyrics` is called for every track in scope — including sidecar-matched tracks (to resolve priority / record conflicts in `source_conflict`) and tracks with no sidecar. No plugin required. On large libraries with many sidecars this adds a substantial number of sequential requests; an info log line shows the embedded-pass count before that loop starts.

### `--lyrics-priority {sidecar,embedded,most_explicit}`

Controls which lyrics source determines the rating when a track has **both** a sidecar file (`.lrc`/`.txt`) and embedded lyrics (requires `--embedded-lyrics`).

| Value | Behaviour |
|---|---|
| `sidecar` *(default)* | Sidecar always wins — use this if you curated your sidecar files deliberately |
| `embedded` | Embedded tag wins when sources disagree — ties (equal tiers) still defer to sidecar |
| `most_explicit` | Whichever source detected the higher tier wins (R > PG-13 > clean) — recommended for maximum protection |

When the two sources disagree, the `source_conflict` column in the CSV report shows which source lost and what it detected, e.g. `sidecar:PG-13->EMBEDDED:R`.

**`[detection.g_genres]`** — optional genre-based G rating. Any audio item whose `Genres` field contains a listed entry (matched **case-insensitively**) and has no matching sidecar file will receive a `G` rating. Omitting the section or leaving `genres = []` disables the feature entirely.

```toml
[detection.g_genres]
genres = ["Classical", "Ambient", "Instrumental", "Chiptune"]
```

Run `--list-genres` to see all genre strings present in your library.

### Detection Details

**Partial-word matching** catches a word even when it's part of a longer word — for example, the stem `ass` matches `badass` or `jackass`. A false-positive list prevents innocent words that happen to contain the same letters (like `class` or `grass`) from triggering.

**Exact matching** is used for shorter words where partial matching would cause too many false positives (e.g., `hoe`, `piss`).

If a track triggers both tiers, R always wins over PG-13.

### CSV Report

The `--report` flag produces a CSV with columns useful for admin review:

| Column | Description |
|--------|-------------|
| `artist` | From server metadata (`AlbumArtist`), falls back to directory structure |
| `album` | From server metadata (`Album`), falls back to directory structure |
| `track` | Audio filename |
| `sidecar` | Sidecar filename (empty for embedded or genre-pass rows) |
| `tier` | `R`, `PG-13`, `G` (genre-matched), or empty (clean) |
| `matched_words` | Semicolon-separated list of words that triggered detection |
| `previous_rating` | What `OfficialRating` was before this run |
| `action` | `set` · `cleared` · `already_correct` · `skipped` · `not_found_in_server` · `server_unavailable` · `no_audio_file` · `error` · `dry_run` · `dry_run_clear` · `g_genre` · `g_genre_already_correct` · `dry_run_g_genre` |
| `source` | `sidecar` · `embedded` · `genre` · `force` — identifies which detection pass produced the row |
| `source_conflict` | Non-empty when sidecar and embedded lyrics disagree; format: `{loser}:{tier}->{WINNER}:{tier}` (e.g. `sidecar:PG-13->EMBEDDED:R`). Loser is lowercase, winner is uppercase; tier is `R`, `PG-13`, or `clean`. Empty when sources agree or only one source was in scope. |

This lets an admin spot false positives caused by lyric transcription errors (e.g., "cuming" instead of "coming") and take corrective action on the sidecar files.
