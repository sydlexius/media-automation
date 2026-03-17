# media-automation

Automation tools for home media servers.

## Tools

### [smpr](smpr/) — Set Music Parental Rating

Automatically rate music tracks on [Emby](https://emby.media/) and
[Jellyfin](https://jellyfin.org/) servers based on lyrics content. Fetches
lyrics from your server, detects explicit language using tiered word detection
(R / PG-13 / G), and sets `OfficialRating` on each track.

- Interactive setup wizard and TUI config editor
- Multi-server support (Emby and Jellyfin simultaneously)
- Per-library and per-location force-rating overrides
- Customizable detection word lists and genre allow-lists
- CSV reporting and dry-run mode
- Cross-platform: Linux, macOS, Windows

Pre-built binaries on [GitHub Releases](https://github.com/sydlexius/media-automation/releases).
