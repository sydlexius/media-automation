# Emby / Jellyfin Scripts

Standalone Python utilities for managing an [Emby](https://emby.media/) or [Jellyfin](https://jellyfin.org/) media server.

## Scripts

| Script | Description |
|--------|-------------|
| [SetMusicParentalRating](SetMusicParentalRating/) | Scans sidecar lyric files for explicit content and sets `OfficialRating` on matching audio tracks via the Emby or Jellyfin API |

## Setup

All scripts share server credentials via a `.env` file in this directory:

```bash
cp .env.example .env
# edit .env → set EMBY_URL + EMBY_API_KEY (Emby), or JELLYFIN_URL + JELLYFIN_API_KEY (Jellyfin)
# If both are set, also add SERVER_TYPE=emby or SERVER_TYPE=jellyfin to disambiguate
```

For production, create a `.env.prod` alongside `.env` with your production server credentials. Scripts accept `--env-file .env.prod` to load it.

Each script has its own subdirectory with a `README.md` and any script-specific configuration or support files (for example, `explicit_config.example.toml` and a `tests/` directory where applicable).
