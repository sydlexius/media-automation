# Emby Scripts

Standalone Python utilities for managing an [Emby](https://emby.media/) media server.

## Scripts

| Script | Description |
|--------|-------------|
| [TagExplicitLyrics](TagExplicitLyrics/) | Scans sidecar lyric files for explicit content and sets `OfficialRating` on matching audio tracks via the Emby API |

## Setup

All scripts share Emby credentials via a `.env` file in this directory:

```bash
cp .env.example .env
# edit .env → set EMBY_API_KEY and EMBY_URL
```

For production, create a `.env.prod` alongside `.env` with your production server credentials. Scripts accept `--env-file .env.prod` to load it.

Each script has its own subdirectory with a `README.md` and any script-specific configuration or support files (for example, `explicit_config.example.toml` and a `tests/` directory where applicable).
