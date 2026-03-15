# Config Struct + TOML/.env Loading — Design Spec

**Date:** 2026-03-14
**Status:** Draft
**Scope:** Issues #67 (Config struct + TOML/.env loading) and #68 (CLI argument parsing — already scaffolded, wiring into config)
**Milestone:** Rust Rewrite: Foundation

---

## Problem

The Rust scaffold (`smpr/`) has CLI parsing via clap derive but no config loading.
The program needs to read a TOML config file, load API keys from `.env`, merge
with CLI flags, and produce a single resolved config for the rest of the program.

## Approach

Two distinct types (Approach C):

1. **RawConfig** — mirrors the TOML file shape exactly. All fields `Option`.
   Used only for deserialization.
2. **Config** — fully resolved. No `Option` for required data. Constructed by
   `Config::load()` which applies merge precedence and validates.

Config loading does **not** touch the network. Server type auto-detection
happens later when the server module connects.

---

## TOML File Shape

```toml
# --- Servers (per-server library/location settings) ---

[servers.home-emby]
url = "http://localhost:8096"
# type = "jellyfin"  # optional; auto-detected if omitted

[servers.home-emby.libraries."Classical Music"]
force_rating = "G"

[servers.home-emby.libraries."Music".locations.classical]
force_rating = "G"

[servers.home-jellyfin]
url = "http://localhost:8097"

[servers.home-jellyfin.libraries."Music"]
force_rating = "G"

# --- Detection (global — same rules for all servers) ---

[detection.r]
stems = ["fuck", "shit", "pussy", "cock", "cum", "faggot"]
exact = ["blowjob", "cocksucker", "motherfuck", "bullshit"]

[detection.pg13]
stems = ["bitch", "whore", "slut"]
exact = ["hoe", "asshole", "piss"]

[detection.ignore]
false_positives = ["cockatoo", "cockatiel", "cocktail", ...]

[detection.g_genres]
genres = ["Ambient", "Classical", "New Age", ...]

# --- General ---

[general]
overwrite = true   # default re-rate behavior (true = overwrite, false = skip)

[report]
output_path = "explicit_report.csv"
```

### Key design decisions

- **Per-server library/location settings.** Libraries are server-specific
  entities. `force_rating` can be set at the library level or the location
  level (or both — location overrides library for that location's tracks;
  library-level force applies to all other locations in the library).
- **Detection is global.** Word lists and genre allow-lists are about language,
  not server-specific. One set of rules applies everywhere.
- **Detection fields resolve independently.** If TOML provides `[detection.r]`
  but omits `[detection.pg13]`, R uses TOML values and PG-13 uses hardcoded
  defaults. Partial overrides are supported.
- **No `library_path`.** The Python-era filesystem path is gone. Library
  scoping is CLI-only (`--library`, `--location`).
- **No legacy `[emby]`/`[jellyfin]` sections.** The named server model
  (`[servers.*]`) replaces the old per-platform config.

---

## .env File Shape

```bash
# {LABEL_UPPER}_API_KEY where label = TOML section name, hyphens → underscores
HOME_EMBY_API_KEY=your-key
HOME_JELLYFIN_API_KEY=your-key
```

Loaded via `dotenvy`. Precedence: `os::env` (real env var) > `.env` file.

---

## Merge Precedence

```
CLI flags > os::env > .env file > TOML file > hardcoded defaults
```

What comes from where:

| Field | CLI | Env | TOML | Default |
|-------|-----|-----|------|---------|
| servers | `--server-url` + `--api-key` (one-off) | API keys | `[servers.*]` | — (required) |
| overwrite | `--overwrite` / `--skip-existing` | — | `[general].overwrite` | `true` |
| dry_run | `--dry-run` | — | — | `false` |
| report_path | `--report` | — | `[report].output_path` | `None` |
| library_name | `--library` | — | — | `None` |
| location_name | `--location` | — | — | `None` |
| verbose | `-v` / `--verbose` | — | — | `false` |
| ignore_forced | `--ignore-forced` | — | — | `false` |
| config_path | `--config` | — | — | `explicit_config.toml` |
| env_file | `--env-file` | — | — | `.env` |
| detection.* | — | — | `[detection.*]` | hardcoded word lists |
| libraries.* | — | — | `[servers.*.libraries.*]` | empty |
| server filter | `--server NAME` (repeatable) | — | — | all configured |

---

## Force Rating Behavior

Per-library and per-location `force_rating` values are stored on the resolved
`ServerConfig` as nested maps:

```
ServerConfig
  └── libraries: { "Music" → LibraryConfig }
        ├── force_rating: Option<"G">
        └── locations: { "classical" → LocationConfig }
              └── force_rating: Option<"G">
```

**Lookup precedence:** location `force_rating` > library `force_rating` > None.

If a track's location matches a location with `force_rating`, that takes
priority. Otherwise, the library-level `force_rating` applies. If neither is
set, normal lyrics evaluation proceeds.

**Interaction with `rate` subcommand:** During `rate`, if the active
library/location has a `force_rating`, that rating is applied to all tracks
in scope without lyrics evaluation (equivalent to `force` subcommand behavior),
unless `--ignore-forced` is passed, which suppresses all force_ratings and
evaluates lyrics normally.

**One-off servers** (`--server-url` + `--api-key`) are assigned the label `cli`
and have no library/location config — force_rating does not apply.

---

## Config Loading Flow

`Config::load()` is called only for `Rate`, `Force`, and `Reset` subcommands.
The `Configure` subcommand handles its own config file discovery and does not
use `Config::load()`.

`Config::load(common: &CommonOpts, ...)` performs:

1. Resolve TOML path: `--config` or default `explicit_config.toml` relative to
   the working directory (`std::env::current_dir()`)
2. Deserialize TOML → `RawConfig` (all `Option` fields)
3. Load `.env` file via `dotenvy` (`--env-file` or default `.env` relative to
   working directory)
4. **Resolve servers:**
   - If `--server-url` + `--api-key` → single one-off server (label=`cli`),
     skip TOML servers. TOML detection/general settings still loaded.
   - Otherwise, iterate `[servers.*]` from TOML:
     - Look up `{LABEL_UPPER}_API_KEY` from env (real env > .env)
     - Error if URL missing or API key not found
     - If `type` is specified in TOML, validate it is `emby` or `jellyfin`
       (else error). Store as `Option<ServerType>`. If omitted, store `None` —
       auto-detection fills it in later (server module, not config).
   - If `--server NAME` filter specified (repeatable), keep only matching
     servers. Error if any named server doesn't exist in TOML, listing
     available servers.
   - Error if no servers resolved
5. **Resolve detection:** Each detection field independently falls back to
   hardcoded defaults if not present in TOML
6. **Resolve general settings:** CLI overwrite/skip flag > TOML
   `[general].overwrite` > default `true`
7. **Resolve report:** CLI `--report` > TOML `[report].output_path` > `None`
8. Return `Config` with all CLI-only pass-through fields (dry_run, verbose,
   library_name, location_name, ignore_forced)

### Scoping notes

- `--location` without `--library` is valid. At runtime, all music libraries
  are searched for a matching location name.

### Error handling

All errors during config loading are fatal (print message to stderr, exit 1).
No partial configs, no fallback-and-continue. If the user's config is broken,
tell them immediately.

Specific errors:
- TOML file specified via `--config` but doesn't exist → error
- Default TOML path doesn't exist → warn, continue with defaults only
- `.env` file specified via `--env-file` but doesn't exist → error
- Server in TOML has no `url` → error
- Server in TOML has no matching API key in env → error
- Server in TOML has invalid `type` value → error
- `--server NAME` references unknown server → error listing available servers
- No servers configured at all → error with guidance

---

## CLI (already scaffolded — #68)

The scaffold in `main.rs` already has the full clap derive setup:

- `CommonOpts` — shared across rate/force/reset (library, location, server,
  dry_run, report, config, env_file, server_url, api_key, verbose)
- `OverwriteOpts` — overwrite/skip_existing with conflict detection
- `Commands` — Rate, Force, Reset, Configure subcommands
- `Rate` includes `--ignore-forced` flag

Remaining work for #68: wire the parsed `Cli` struct into `Config::load()` so
the args flow into config resolution. This is part of #67's implementation.

---

## What's NOT in scope

- Server type auto-detection (Milestone 6: Server API Client)
- Network calls of any kind
- Detection engine (Milestone 7)
- Rating orchestration (Milestone 8)
- Configure wizard / TUI (Milestone 9)

---

## Testing

Unit tests in `config.rs`:

1. Parse a valid TOML string → verify `RawConfig` fields
2. Parse minimal TOML (only `[servers.*]`) → verify defaults applied
3. Parse TOML with per-library force_rating → verify library config
4. Parse TOML with per-location force_rating → verify nesting
5. Missing API key → verify error message
6. One-off `--server-url` + `--api-key` with valid TOML → verify single server
   resolved (label=`cli`), TOML servers ignored, detection/general still loaded
7. `--server NAME` filter → verify only named servers kept
8. Overwrite precedence: CLI flag > TOML > default
9. Empty/missing TOML file → verify defaults
10. TOML with unknown fields (top-level and nested) → verify serde ignores them gracefully (no `deny_unknown_fields`)
11. Invalid `type` in server TOML → verify error
12. `--server NAME` with unknown name → verify error lists available servers
