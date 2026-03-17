# smpr — Set Music Parental Rating

CLI tool for [Emby](https://emby.media/) and [Jellyfin](https://jellyfin.org/) media servers. Fetches lyrics, detects explicit content using tiered word detection (R / PG-13), and sets parental ratings on audio tracks.

## Installation

### Download

Pre-built binaries are available on [GitHub Releases](https://github.com/sydlexius/media-automation/releases):

| Platform | Binary |
| -------- | ------ |
| Linux (x86_64, static) | `smpr-linux-x86_64` |
| macOS (Intel) | `smpr-macos-intel` |
| macOS (Apple Silicon) | `smpr-macos-apple-silicon` |
| Windows (x86_64) | `smpr-windows-x86_64.exe` |

### Build from source

```bash
cd smpr
cargo build --release
# Binary at: target/release/smpr
```

## Quick Start

```bash
# Interactive setup wizard — creates config + .env
smpr configure

# Analyze a library without making changes
smpr rate --library Music --dry-run

# Set ratings based on lyrics analysis
smpr rate --library Music

# Force-rate a known-clean library
smpr force G --library "Classical Music"

# Remove all ratings from a library
smpr reset --library Music

# Generate a CSV report
smpr rate --library Music --dry-run --report report.csv
```

For full options: `smpr --help` or `smpr <subcommand> --help`.

## Configuration

Run `smpr configure` for an interactive setup wizard — recommended for first-time users. It will create `explicit_config.toml` and a `.env` file with your API keys.

For manual setup:

- **Config file**: `explicit_config.toml` in the current directory, or the platform config directory (e.g., `~/.config/smpr/config.toml` on Linux, `~/Library/Application Support/smpr/config.toml` on macOS)
- **API keys**: stored in `.env` as `{LABEL}_API_KEY` (e.g., `HOME_EMBY_API_KEY` for a server named `home-emby`)
- **Precedence**: CLI flags > env vars > `.env` file > TOML config > defaults

Example config structure:

```toml
[servers.home-emby]
url = "http://192.168.1.126:8096"
# type = "emby"  # optional; auto-detected if omitted

[servers.home-emby.libraries.Music]
# force_rating = "G"  # optional; force all tracks in this library to G
```

```bash
# .env
HOME_EMBY_API_KEY=your-api-key-here
```

API keys never go in the TOML file — only in `.env`.

## Development

```bash
cd smpr
cargo test --verbose -- --test-threads=1   # tests must run sequentially
cargo fmt -- --check && cargo clippy -- -D warnings
cargo build --release
```
