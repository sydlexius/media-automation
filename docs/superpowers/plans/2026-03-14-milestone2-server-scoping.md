# Milestone 2: Server & Scoping — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the per-platform env var scheme with auto-detecting named servers, add library/location scoping via the server API, rename subcommands to match API-driven workflow, and add overwrite/skip-existing behavior.

**Architecture:** All changes modify `SetMusicParentalRating/SetMusicParentalRating.py` (single-file script, ~1620 lines). A new `ServerConfig` dataclass holds per-server info. `detect_server_type()` auto-detects Emby/Jellyfin via `/System/Info/Public`. `build_config` gains a new code path to parse TOML `[servers.*]` sections with `{LABEL}_API_KEY` env vars, falling back to old env vars during transition. `main()` iterates over resolved servers, replacing the current "both" mode. Library/location scoping uses `ParentId` from `/Library/VirtualFolders` to filter the bulk prefetch query.

**Tech Stack:** Python 3.11+ stdlib only (urllib, json, argparse, tomllib, dataclasses)

**Spec:** `docs/superpowers/specs/2026-03-13-api-driven-refactor-design.md` — Milestone 2 section

---

## File Structure

All PRs modify the same primary file. Config templates and docs are updated at PR boundaries.

| File | PR(s) | Role |
|------|-------|------|
| `SetMusicParentalRating/SetMusicParentalRating.py` | A–E | Script (all logic) |
| `.env.example` | A, B | Credentials template |
| `SetMusicParentalRating/explicit_config.example.toml` | A, B, C | Config template |
| `CLAUDE.md` | E (final) | Architecture docs |

## Branching & PR Strategy

From the spec's Implementation Strategy:

| PR | Issues | Branch | Strategy |
|----|--------|--------|----------|
| **A** | #37 + #38 | `feat/auto-detect-named-servers` | Regular branch, one PR (tightly coupled) |
| **B** | #39 | Graphite off A | Remove old env var scheme |
| **C** | #40 | Graphite off A | Library/location discovery |
| **D** | #41 | Graphite off C | Rename subcommands |
| **E** | #42 | Graphite off D | Overwrite/skip-existing |

Dependency graph: `A → B`, `A → C → D → E`. PRs B and C are independent after A merges.

**Graphite usage:** Stack C → D → E as one Graphite stack. PR B can be an independent branch or a second single-branch stack off A.

---

## Chunk 1: PR A — Auto-detect + Named Server Model (#37, #38)

### Task 1: Add `ServerConfig` dataclass and `detect_server_type()`

**Files:**
- Modify: `SetMusicParentalRating/SetMusicParentalRating.py:98-157` (Dataclasses section)

- [ ] **Step 1: Add `ServerConfig` dataclass**

Insert after the `MediaServerError` class (before `Config`):

```python
@dataclass
class ServerConfig:
    """Named server configuration — one per [servers.*] TOML section."""

    name: str
    url: str
    api_key: str
    server_type: str = ""  # auto-detected via /System/Info/Public
```

- [ ] **Step 2: Add `servers` field to `Config`**

Add to the `Config` dataclass (after `jellyfin_api_key`):

```python
    servers: list[ServerConfig] = field(default_factory=list)
```

Update the `server_type` validation in `Config.__post_init__` to drop `"both"` (the multi-server concept is now handled by the `servers` list, not `server_type`):

```python
        if self.server_type not in ("emby", "jellyfin"):
            raise ValueError(
                f"server_type must be 'emby' or 'jellyfin', got {self.server_type!r}"
            )
```

Keep the regex precompilation and `g_genres` stripping.

- [ ] **Step 3: Add `detect_server_type()` function**

Insert in the Configuration section (after `load_env`, before `build_config`):

```python
def detect_server_type(url: str) -> str:
    """Auto-detect server type via GET /System/Info/Public (unauthenticated).

    Returns ``"emby"`` or ``"jellyfin"``.

    Primary: ``ProductName == "Jellyfin Server"`` → Jellyfin; else Emby.
    Fallback: ``Server`` response header — ``Kestrel`` → Jellyfin; else Emby.
    Raises ``MediaServerError`` if the endpoint is unreachable.
    """
    clean_url = url.rstrip("/")
    endpoint = f"{clean_url}/System/Info/Public"
    req = urllib.request.Request(endpoint)
    req.add_header("Accept", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            server_header = resp.headers.get("Server", "")
            try:
                data = json.loads(resp.read())
                if data.get("ProductName") == "Jellyfin Server":
                    log.info("Auto-detected Jellyfin at %s", clean_url)
                    return "jellyfin"
                log.info("Auto-detected Emby at %s", clean_url)
                return "emby"
            except (json.JSONDecodeError, AttributeError):
                pass
            # Fallback: Server header
            if "Kestrel" in server_header:
                log.info("Auto-detected Jellyfin (via Server header) at %s", clean_url)
                return "jellyfin"
            log.info("Auto-detected Emby (via Server header fallback) at %s", clean_url)
            return "emby"
    except (urllib.error.URLError, OSError) as exc:
        raise MediaServerError(
            f"Cannot reach {endpoint} to auto-detect server type: {exc}"
        ) from exc
```

- [ ] **Step 4: Verify import**

Run: `.venv/bin/python3 -c "import SetMusicParentalRating"` (from `SetMusicParentalRating/`)

Expected: no errors

- [ ] **Step 5: Run ruff**

Run: `ruff check . && ruff format --check .`

Expected: clean

---

### Task 2: Add server resolution to `build_config`

**Files:**
- Modify: `SetMusicParentalRating/SetMusicParentalRating.py:222-394` (`build_config` function)

- [ ] **Step 1: Add `_resolve_servers()` helper**

Insert before `build_config`:

```python
def _resolve_servers(
    args: argparse.Namespace,
    toml: dict,
    env_file: dict[str, str],
) -> list[ServerConfig]:
    """Resolve server list from CLI overrides, TOML [servers.*], or legacy env vars.

    Precedence:
    1. ``--server-url`` + ``--api-key`` → single one-off server
    2. TOML ``[servers.*]`` sections → named servers
    3. Legacy ``EMBY_URL``/``JELLYFIN_URL`` env vars → synthetic named servers

    For each server without an explicit ``type``, auto-detects via
    ``/System/Info/Public``.
    """
    cli_server_url = (getattr(args, "server_url", None) or "").strip()
    cli_api_key = (getattr(args, "api_key", None) or "").strip()

    # --- 1. CLI one-off override ---
    if cli_server_url and cli_api_key:
        server_type = detect_server_type(cli_server_url)
        return [
            ServerConfig(
                name="cli",
                url=cli_server_url.rstrip("/"),
                api_key=cli_api_key,
                server_type=server_type,
            )
        ]
    if cli_server_url or cli_api_key:
        print(
            "Error: --server-url and --api-key must both be provided together.",
            file=sys.stderr,
        )
        sys.exit(1)

    # --- 2. TOML [servers.*] sections ---
    toml_servers = toml.get("servers", {})
    if toml_servers and isinstance(toml_servers, dict):
        servers: list[ServerConfig] = []
        for name, srv_conf in toml_servers.items():
            if not isinstance(srv_conf, dict):
                continue
            url = str(srv_conf.get("url", "")).strip()
            if not url:
                print(
                    f"Error: server '{name}' in config has no 'url'.",
                    file=sys.stderr,
                )
                sys.exit(1)
            # API key: {LABEL_UPPER}_API_KEY (hyphens → underscores)
            env_key_name = f"{name.upper().replace('-', '_')}_API_KEY"
            api_key = (
                os.environ.get(env_key_name, "")
                or env_file.get(env_key_name, "")
            ).strip()
            if not api_key:
                print(
                    f"Error: no API key for server '{name}'. "
                    f"Set {env_key_name} in your .env file.",
                    file=sys.stderr,
                )
                sys.exit(1)
            # Server type: explicit override or auto-detect
            explicit_type = str(srv_conf.get("type", "")).strip().lower()
            if explicit_type:
                if explicit_type not in ("emby", "jellyfin"):
                    print(
                        f"Error: server '{name}' has invalid type "
                        f"'{explicit_type}' (must be 'emby' or 'jellyfin').",
                        file=sys.stderr,
                    )
                    sys.exit(1)
                server_type = explicit_type
            else:
                server_type = detect_server_type(url)
            servers.append(
                ServerConfig(
                    name=name,
                    url=url.rstrip("/"),
                    api_key=api_key,
                    server_type=server_type,
                )
            )
        if servers:
            return servers

    # --- 3. Legacy env var fallback ---
    emby_url = (
        os.environ.get("EMBY_URL", "")
        or env_file.get("EMBY_URL", "")
        or str(toml.get("emby", {}).get("url", "") or "")
    ).strip()
    emby_api_key = (
        os.environ.get("EMBY_API_KEY", "") or env_file.get("EMBY_API_KEY", "")
    ).strip()
    jellyfin_url = (
        os.environ.get("JELLYFIN_URL", "")
        or env_file.get("JELLYFIN_URL", "")
        or str(toml.get("jellyfin", {}).get("url", "") or "")
    ).strip()
    jellyfin_api_key = (
        os.environ.get("JELLYFIN_API_KEY", "")
        or env_file.get("JELLYFIN_API_KEY", "")
    ).strip()

    # Respect explicit --server-type during transition (PR B removes this)
    explicit_type = (
        (getattr(args, "server_type", None) or "")
        or os.environ.get("SERVER_TYPE", "")
        or env_file.get("SERVER_TYPE", "")
        or str(toml.get("general", {}).get("server_type", "") or "")
    ).lower().strip()

    servers = []
    if explicit_type == "both":
        if emby_url and emby_api_key:
            servers.append(
                ServerConfig("emby", emby_url.rstrip("/"), emby_api_key, "emby")
            )
        if jellyfin_url and jellyfin_api_key:
            servers.append(
                ServerConfig(
                    "jellyfin",
                    jellyfin_url.rstrip("/"),
                    jellyfin_api_key,
                    "jellyfin",
                )
            )
        if len(servers) < 2:
            missing = []
            if not emby_url or not emby_api_key:
                missing.append("EMBY_URL/EMBY_API_KEY")
            if not jellyfin_url or not jellyfin_api_key:
                missing.append("JELLYFIN_URL/JELLYFIN_API_KEY")
            print(
                f"Error: --server-type both requires {' and '.join(missing)}.",
                file=sys.stderr,
            )
            sys.exit(1)
    elif explicit_type == "jellyfin":
        url = jellyfin_url
        key = jellyfin_api_key
        if not url or not key:
            print(
                "Error: Jellyfin requires JELLYFIN_URL and JELLYFIN_API_KEY.",
                file=sys.stderr,
            )
            sys.exit(1)
        servers.append(ServerConfig("jellyfin", url.rstrip("/"), key, "jellyfin"))
    elif explicit_type == "emby":
        url = emby_url
        key = emby_api_key
        if not url or not key:
            print(
                "Error: Emby requires EMBY_URL and EMBY_API_KEY.",
                file=sys.stderr,
            )
            sys.exit(1)
        servers.append(ServerConfig("emby", url.rstrip("/"), key, "emby"))
    else:
        # Auto-detect: use whichever is configured
        if emby_url and emby_api_key:
            stype = detect_server_type(emby_url) if not explicit_type else "emby"
            servers.append(
                ServerConfig("emby", emby_url.rstrip("/"), emby_api_key, stype)
            )
        if jellyfin_url and jellyfin_api_key:
            stype = (
                detect_server_type(jellyfin_url) if not explicit_type else "jellyfin"
            )
            servers.append(
                ServerConfig(
                    "jellyfin",
                    jellyfin_url.rstrip("/"),
                    jellyfin_api_key,
                    stype,
                )
            )
        if not servers:
            print(
                "Error: no server configured. Set server credentials in .env "
                "or add [servers.*] sections to the TOML config.",
                file=sys.stderr,
            )
            sys.exit(1)
        if len(servers) > 1 and not explicit_type:
            print(
                "Error: multiple servers configured. Use --server-type emby, "
                "--server-type jellyfin, --server-type both, or migrate to "
                "[servers.*] TOML sections and use --server NAME to select.",
                file=sys.stderr,
            )
            sys.exit(1)

    return servers
```

- [ ] **Step 2: Simplify `build_config` to use `_resolve_servers()`**

Replace the server resolution block in `build_config` (lines 277–394 — from the `# --- server_type` comment through the closing `return Config(...)` block). **Keep the library_paths resolution block (lines 242–275) intact.** The replacement code below starts immediately after the library_paths block:

```python
    # --- Resolve servers ---
    servers = _resolve_servers(args, toml, env_file)

    # --- Filter by --server NAME (if specified) ---
    selected = getattr(args, "server", None) or []
    if selected:
        known = {s.name for s in servers}
        for name in selected:
            if name not in known:
                avail = ", ".join(sorted(known))
                print(
                    f"Error: unknown server '{name}'. Available: {avail}",
                    file=sys.stderr,
                )
                sys.exit(1)
        servers = [s for s in servers if s.name in selected]

    # Backward-compat: set server_url/api_key/type from first resolved server
    active = servers[0] if servers else ServerConfig("", "", "", "emby")

    # --- word lists (TOML or defaults) ---
    det = toml.get("detection", {})
    r_stems = det.get("r", {}).get("stems", list(DEFAULT_R_STEMS))
    r_exact = det.get("r", {}).get("exact", list(DEFAULT_R_EXACT))
    pg13_stems = det.get("pg13", {}).get("stems", list(DEFAULT_PG13_STEMS))
    pg13_exact = det.get("pg13", {}).get("exact", list(DEFAULT_PG13_EXACT))
    false_positives = det.get("ignore", {}).get(
        "false_positives", list(DEFAULT_FALSE_POSITIVES)
    )
    g_genres = det.get("g_genres", {}).get("genres", [])
    # --- report ---
    report_path_str = getattr(args, "report", None) or toml.get("report", {}).get(
        "output_path"
    )
    report_path = Path(report_path_str) if report_path_str else None

    return Config(
        library_paths=library_paths,
        server_url=active.url,
        server_api_key=active.api_key,
        server_type=active.server_type,
        r_stems=r_stems,
        r_exact=r_exact,
        pg13_stems=pg13_stems,
        pg13_exact=pg13_exact,
        false_positives=false_positives,
        dry_run=getattr(args, "dry_run", False),
        clear=getattr(args, "clear", False),
        force_rating=getattr(args, "rating", None),
        report_path=report_path,
        g_genres=g_genres,
        emby_url=active.url if active.server_type == "emby" else "",
        emby_api_key=active.api_key if active.server_type == "emby" else "",
        jellyfin_url=active.url if active.server_type == "jellyfin" else "",
        jellyfin_api_key=active.api_key if active.server_type == "jellyfin" else "",
        servers=servers,
    )
```

- [ ] **Step 3: Verify import and ruff**

Run: `cd SetMusicParentalRating && ../.venv/bin/python3 -c "import SetMusicParentalRating" && cd .. && ruff check . && ruff format --check .`

Expected: clean

---

### Task 3: Add `--server` CLI flag and update `main()`

**Files:**
- Modify: `SetMusicParentalRating/SetMusicParentalRating.py` — `build_parser()` and `main()`

- [ ] **Step 1: Add `--server` to shared parser**

In `build_parser()`, add to the `shared` parser (after `--api-key`):

```python
    shared.add_argument(
        "--server",
        action="append",
        default=None,
        dest="server",
        metavar="NAME",
        help="Target a named server (repeatable; e.g. --server home-emby --server home-jellyfin)",
    )
```

- [ ] **Step 2: Update help text and examples**

Update `_MAIN_EXAMPLES` to reflect the new `--server` flag:

```python
_MAIN_EXAMPLES = """\
subcommands:
  scan    Fetch lyrics from server, detect explicit content, set ratings
  rate    Set a fixed rating on all tracks under the given path(s)
  genres  List all Audio genre tags from the server

examples:
  %(prog)s scan /path/to/music --dry-run --report report.csv
  %(prog)s scan --server home-emby --dry-run
  %(prog)s rate /path/to/classical G
  %(prog)s genres
"""
```

- [ ] **Step 3: Rewrite `main()` to iterate over resolved servers**

Replace the entire `main()` function:

```python
def main() -> None:
    parser = build_parser()
    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        sys.exit(0)

    setup_logging(args.verbose)

    try:
        config = build_config(args)
    except ValueError as exc:
        parser.error(str(exc))

    if args.command == "genres":
        list_genres_mode(config)
        return

    all_results: list[DetectionResult] = []
    multi = len(config.servers) > 1

    for server in config.servers:
        srv_config = replace(
            config,
            server_url=server.url,
            server_api_key=server.api_key,
            server_type=server.server_type,
        )
        label = (
            f"{server.name} ({server.server_type.title()})" if multi else ""
        )
        if label:
            log.info("--- Processing %s ---", label)

        # Note: dispatches on config.force_rating (set when `rate` subcommand
        # provides a positional rating arg). PR D replaces this with explicit
        # args.command dispatch when subcommands are renamed.
        if config.force_rating:
            try:
                results = force_rate_library(srv_config)
            except SystemExit as exc:
                log.error(
                    "%s failed (exit %s).",
                    label or "Server",
                    exc.code,
                )
                results = []
        else:
            try:
                results = process_library(srv_config)
            except SystemExit as exc:
                log.error(
                    "%s failed (exit %s).",
                    label or "Server",
                    exc.code,
                )
                results = []

        all_results.extend(results)
        if multi:
            print_summary(results, label=label)

    if config.report_path:
        write_report(all_results, config.report_path, config.library_paths)

    if not multi:
        print_summary(all_results)
```

- [ ] **Step 4: Verify import and ruff**

Run: `cd SetMusicParentalRating && ../.venv/bin/python3 -c "import SetMusicParentalRating" && cd .. && ruff check . && ruff format --check .`

Expected: clean

---

### Task 4: Update config templates

**Files:**
- Modify: `.env.example`
- Modify: `SetMusicParentalRating/explicit_config.example.toml`

- [ ] **Step 1: Update `.env.example`**

Replace contents with:

```bash
# --- Named server credentials (new format) ---
# Each server defined in [servers.*] in explicit_config.toml needs a
# matching {LABEL_UPPER}_API_KEY here (hyphens → underscores).
#
# HOME_EMBY_API_KEY=your-emby-api-key
# HOME_JELLYFIN_API_KEY=your-jellyfin-api-key

# --- Legacy format (still supported; will be removed in a future release) ---
EMBY_API_KEY=your-api-key-here
EMBY_URL=http://localhost:8096

JELLYFIN_URL=http://localhost:8097
JELLYFIN_API_KEY=your-jellyfin-api-key-here

# Required only if BOTH Emby and Jellyfin are configured above.
# When only one server is configured its type is detected automatically.
# SERVER_TYPE=emby
```

- [ ] **Step 2: Update `explicit_config.example.toml`**

Add `[servers.*]` sections (keep old `[emby]`/`[jellyfin]` for transition):

```toml
# Copy this file to explicit_config.toml and edit to taste.
# The script loads explicit_config.toml by default (override with --config).
#
# Precedence: CLI flags > environment variables > .env file > this file > defaults
#
# Environment files:
#   The script loads .env from the repo root by default.
#   Use --env-file to load a different file (e.g. --env-file .env.prod).

# --- Named servers (preferred) ---
# Define one or more servers. API keys go in .env as {LABEL_UPPER}_API_KEY
# (hyphens in label → underscores in env var name).
# Server type (emby/jellyfin) is auto-detected; set type = "emby" or
# type = "jellyfin" to override.
#
# [servers.home-emby]
# url = "http://192.168.1.126:8096"
#
# [servers.home-jellyfin]
# url = "http://192.168.1.126:8097"

# --- Legacy server config (still supported; will be removed) ---
[emby]
url = "http://localhost:8096"

[jellyfin]
url = "http://localhost:8097"

[general]
# server_type = "emby"   # Legacy; auto-detected when using [servers.*]
library_path = ""

[detection]

[detection.r]
stems = ["fuck", "shit", "pussy", "cock", "cum", "faggot"]
exact = ["blowjob", "cocksucker", "motherfuck", "bullshit"]

[detection.pg13]
stems = ["bitch", "whore", "slut"]
exact = ["hoe", "asshole", "piss"]

[detection.ignore]
false_positives = [
    "cockatoo",
    "cockatiel",
    "cocktail",
    "hancock",
    "dickens",
    "dickson",
    "scunthorpe",
    "pissarro",
    "circumstance",
    "circumstan",
    "cucumber",
    "cumulative",
    "cumbersome",
    "cumberbatch",
    "document",
    "incumbent",
    "succumb",
    "accumulate",
]

[detection.g_genres]
genres = []

[report]
output_path = "explicit_report.csv"
```

- [ ] **Step 3: Verify ruff and AST**

Run: `ruff check . && ruff format --check . && cd SetMusicParentalRating && ../.venv/bin/python3 -c "import SetMusicParentalRating"`

---

### Task 5: UAT — Auto-detect + Named servers

**Pre-requisite:** Ensure at least Jellyfin is running at localhost:8097. If Emby is available at localhost:8096, test both.

- [ ] **Step 1: UAT with legacy env vars (backward compat)**

Using the existing `.env` (old `EMBY_URL`/`JELLYFIN_URL` format):

```bash
cd SetMusicParentalRating
../.venv/bin/python3 SetMusicParentalRating.py scan --dry-run -v 2>&1 | head -30
```

Expected: auto-detects server type from env vars, runs scan with `[DRY RUN]` output.

- [ ] **Step 2: UAT with named servers**

Create `explicit_config.uat.toml` (gitignored) with:

```toml
[servers.uat-jellyfin]
url = "http://localhost:8097"

[general]
library_path = "/music"

[detection.r]
stems = ["fuck"]
exact = []
[detection.pg13]
stems = []
exact = []
[detection.ignore]
false_positives = []
```

Add to `.env`: `UAT_JELLYFIN_API_KEY=<your-jellyfin-key>`

```bash
../.venv/bin/python3 SetMusicParentalRating.py scan --config explicit_config.uat.toml --dry-run -v 2>&1 | head -30
```

Expected: auto-detects "jellyfin" from the URL, runs scan against Jellyfin.

- [ ] **Step 3: UAT `--server` flag**

```bash
../.venv/bin/python3 SetMusicParentalRating.py scan --config explicit_config.uat.toml --server uat-jellyfin --dry-run -v 2>&1 | head -20
```

Expected: selects only `uat-jellyfin`, runs scan.

- [ ] **Step 4: UAT `--server-url` + `--api-key` one-off**

```bash
../.venv/bin/python3 SetMusicParentalRating.py scan --server-url http://localhost:8097 --api-key <key> --dry-run -v 2>&1 | head -20
```

Expected: auto-detects jellyfin, runs scan with one-off credentials.

- [ ] **Step 5: UAT error cases**

```bash
# Unknown server name
../.venv/bin/python3 SetMusicParentalRating.py scan --config explicit_config.uat.toml --server nonexistent --dry-run 2>&1

# Only --server-url without --api-key
../.venv/bin/python3 SetMusicParentalRating.py scan --server-url http://localhost:8097 --dry-run 2>&1
```

Expected: clear error messages.

---

### Task 6: Commit and create PR A

- [ ] **Step 1: Commit**

```bash
git checkout -b feat/auto-detect-named-servers
git add SetMusicParentalRating/SetMusicParentalRating.py .env.example SetMusicParentalRating/explicit_config.example.toml
git commit -m "feat: auto-detect server type + named server model (#37, #38)

Add detect_server_type() via GET /System/Info/Public (unauthenticated).
Add ServerConfig dataclass and [servers.*] TOML sections with
{LABEL}_API_KEY env var convention. Add --server NAME flag for
explicit server selection. Rewrite main() to iterate over resolved
servers, replacing the old --server-type both mode.

Legacy env vars (EMBY_URL, JELLYFIN_URL, etc.) still work as fallback.

Closes #37, closes #38"
```

- [ ] **Step 2: Push and create PR**

```bash
git push -u origin feat/auto-detect-named-servers
gh pr create --title "feat: auto-detect server type + named server model (#37, #38)" \
  --body "$(cat <<'EOF'
## Summary
- Auto-detect Emby vs Jellyfin via `GET /System/Info/Public` (unauthenticated). Primary: `ProductName`; fallback: `Server` header.
- Named server model: `[servers.*]` TOML sections + `{LABEL}_API_KEY` env vars replace per-platform env vars.
- `--server NAME` flag (repeatable) for explicit server selection.
- `main()` rewritten to iterate over resolved servers, replacing the old `--server-type both` mode.
- Legacy env vars still work as fallback (removed in next PR).

## Test plan
- [ ] UAT: legacy env var backward compat (scan with old .env)
- [ ] UAT: named server config (new TOML format)
- [ ] UAT: `--server NAME` explicit selection
- [ ] UAT: `--server-url` + `--api-key` one-off
- [ ] UAT: error cases (unknown server, missing key)
- [ ] Import smoke test passes
- [ ] ruff check/format clean

Closes #37, closes #38

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Chunk 2: PR B — Remove Old Server-Type Flag (#39)

### Task 7: Remove `--server-type` flag and legacy env var scheme

**Files:**
- Modify: `SetMusicParentalRating/SetMusicParentalRating.py`

- [ ] **Step 1: Remove `--server-type` from shared parser**

In `build_parser()`, remove the `shared.add_argument("--server-type", ...)` block.

- [ ] **Step 2: Remove legacy env var fields from `Config`**

Remove these fields from the `Config` dataclass:

```python
    emby_url: str = ""
    emby_api_key: str = ""
    jellyfin_url: str = ""
    jellyfin_api_key: str = ""
```

- [ ] **Step 3: Remove legacy fallback from `_resolve_servers()`**

Remove the entire "3. Legacy env var fallback" section from `_resolve_servers()`. Change the function to error if no `[servers.*]` sections are found and no `--server-url`/`--api-key` override is given:

```python
    # --- 3. No servers configured ---
    print(
        "Error: no servers configured. Add [servers.*] sections to the TOML "
        "config, or use --server-url and --api-key for a one-off run.",
        file=sys.stderr,
    )
    sys.exit(1)
```

- [ ] **Step 4: Remove legacy fields from `build_config` return**

In the `Config(...)` constructor call in `build_config`, remove the `emby_url`, `emby_api_key`, `jellyfin_url`, `jellyfin_api_key` keyword arguments.

- [ ] **Step 5: Remove old `[emby]`/`[jellyfin]` TOML parsing and env var references**

Search the entire file (not just `build_config`) for any remaining references to `toml.get("emby", {})`, `toml.get("jellyfin", {})`, `EMBY_URL`, `EMBY_API_KEY`, `JELLYFIN_URL`, `JELLYFIN_API_KEY`. Remove them. In particular, `load_env()` (lines 168-174) checks for `EMBY_URL`/`EMBY_API_KEY`/`JELLYFIN_URL`/`JELLYFIN_API_KEY` in `os.environ` to decide whether to warn about a missing `.env` file — simplify this to just check whether any `*_API_KEY` env vars are set, or remove the special-case logic entirely.

- [ ] **Step 6: Remove `SERVER_TYPE` env var handling**

Remove any references to `SERVER_TYPE` env var in `_resolve_servers()` and `build_config`.

- [ ] **Step 7: Clean up `list_genres_mode`**

Remove the `server_type == "both"` error check in `list_genres_mode()` (no longer applicable — multi-server is handled by `main()`). The function now always operates on a single server's config.

- [ ] **Step 8: Verify import and ruff**

Run: `cd SetMusicParentalRating && ../.venv/bin/python3 -c "import SetMusicParentalRating" && cd .. && ruff check . && ruff format --check .`

---

### Task 8: Update config templates for new-only format

- [ ] **Step 1: Update `.env.example`**

Remove legacy format, keep only named server keys:

```bash
# Each server defined in [servers.*] in explicit_config.toml needs a
# matching {LABEL_UPPER}_API_KEY here (hyphens → underscores).
#
# Example for [servers.home-emby]:
# HOME_EMBY_API_KEY=your-emby-api-key
#
# Example for [servers.home-jellyfin]:
# HOME_JELLYFIN_API_KEY=your-jellyfin-api-key
```

- [ ] **Step 2: Update `explicit_config.example.toml`**

Remove `[emby]`, `[jellyfin]` sections and `server_type` reference. Uncomment the `[servers.*]` examples:

```toml
# Copy this file to explicit_config.toml and edit to taste.
# The script loads explicit_config.toml by default (override with --config).
#
# Precedence: CLI flags > environment variables > .env file > this file > defaults
#
# Environment files:
#   The script loads .env from the repo root by default.
#   Use --env-file to load a different file (e.g. --env-file .env.prod).
#   API keys go in .env as {LABEL_UPPER}_API_KEY (hyphens → underscores).

# --- Servers ---
# Server type (emby/jellyfin) is auto-detected from the server's API.
# Set type = "emby" or type = "jellyfin" to override auto-detection.

[servers.home-emby]
url = "http://localhost:8096"

[servers.home-jellyfin]
url = "http://localhost:8097"

[general]
library_path = ""

[detection]
# ... (unchanged)
```

---

### Task 9: UAT and commit PR B

- [ ] **Step 1: UAT with new format only**

Ensure `.env` has `HOME_EMBY_API_KEY` and/or `HOME_JELLYFIN_API_KEY` (not old `EMBY_API_KEY`). Ensure `explicit_config.toml` has `[servers.*]` sections (not `[emby]`/`[jellyfin]`).

```bash
cd SetMusicParentalRating
../.venv/bin/python3 SetMusicParentalRating.py scan --dry-run -v 2>&1 | head -30
```

Expected: works with new format, no legacy env var references.

- [ ] **Step 2: UAT error — old format gives clear error**

With old `.env` format (only `EMBY_API_KEY`, no `[servers.*]` TOML):

```bash
../.venv/bin/python3 SetMusicParentalRating.py scan --dry-run 2>&1
```

Expected: "Error: no servers configured. Add [servers.*] sections..."

- [ ] **Step 3: Commit**

```bash
git add SetMusicParentalRating/SetMusicParentalRating.py .env.example SetMusicParentalRating/explicit_config.example.toml
git commit -m "feat: remove --server-type flag and legacy env var scheme (#39)

Remove --server-type CLI flag, SERVER_TYPE env var, EMBY_URL/EMBY_API_KEY/
JELLYFIN_URL/JELLYFIN_API_KEY env vars, and [emby]/[jellyfin] TOML sections.
Named servers ([servers.*] + {LABEL}_API_KEY) are now the only supported
configuration format.

Closes #39"
```

- [ ] **Step 4: Push and create PR**

Use Graphite to stack off PR A:

```bash
gt create -m "feat: remove --server-type flag and legacy env var scheme (#39)"
gt stack submit
```

---

## Chunk 3: PR C — Library/Location Discovery (#40)

### Task 10: Add library discovery to `MediaServerClient`

**Files:**
- Modify: `SetMusicParentalRating/SetMusicParentalRating.py` — `MediaServerClient` class

- [ ] **Step 1: Add `discover_libraries()` method**

Add to `MediaServerClient` (after `list_genres`):

```python
    def discover_libraries(self) -> list[dict]:
        """GET /Library/VirtualFolders — return music libraries.

        Each entry has ``Name``, ``ItemId``, ``Locations`` (list of path
        strings), and ``CollectionType``.  Only libraries with
        ``CollectionType == "music"`` are returned.
        """
        result = self._request("GET", "/Library/VirtualFolders")
        if not result or not isinstance(result, list):
            log.warning("discover_libraries: unexpected response: %r", type(result))
            return []
        music_libs = [
            lib
            for lib in result
            if isinstance(lib, dict) and lib.get("CollectionType") == "music"
        ]
        log.info(
            "Discovered %d music library/libraries: %s",
            len(music_libs),
            ", ".join(lib.get("Name", "?") for lib in music_libs),
        )
        return music_libs
```

- [ ] **Step 2: Verify import and ruff**

---

### Task 11: Add `--library` and `--location` flags, scope prefetch

**Files:**
- Modify: `SetMusicParentalRating/SetMusicParentalRating.py` — `build_parser()`, `prefetch_audio_items()`, `process_library()`, `force_rate_library()`

- [ ] **Step 1: Add CLI flags to shared parser**

In `build_parser()`, add to the `shared` parser:

```python
    shared.add_argument(
        "--library",
        default=None,
        metavar="NAME",
        help="Scope to a specific music library (default: all music libraries)",
    )
    shared.add_argument(
        "--location",
        default=None,
        metavar="NAME",
        help="Scope to a location within a library (e.g. 'classical')",
    )
```

- [ ] **Step 2: Add `parent_id` parameter to `prefetch_audio_items()`**

Modify the method signature and query:

```python
    def prefetch_audio_items(
        self,
        *,
        include_media_sources: bool = False,
        parent_id: str | None = None,
    ) -> dict[str, dict]:
```

In the query string, add `ParentId` when provided:

```python
            parent_filter = f"&ParentId={parent_id}" if parent_id else ""
            result = self._request(
                "GET",
                f"/Users/{uid}/Items?Recursive=true&IncludeItemTypes=Audio"
                f"&Fields={fields}{parent_filter}"
                f"&StartIndex={start_index}&Limit={page_size}",
            )
```

- [ ] **Step 3: Add `library_name` and `location_name` fields to `Config`**

Add to the `Config` dataclass (after `g_genres`):

```python
    library_name: str | None = None
    location_name: str | None = None
```

- [ ] **Step 4: Add `_resolve_library_scope()` helper**

Insert before `process_library`. This function returns a tuple: `(parent_id, location_path)`.
The `parent_id` scopes the prefetch query to a library. The `location_path` (when `--location` is specified) is used for post-prefetch filtering since locations don't have their own item IDs in the VirtualFolders response.

```python
def _resolve_library_scope(
    client: MediaServerClient,
    library_name: str | None,
    location_name: str | None,
) -> tuple[str | None, str | None]:
    """Resolve --library/--location to a ParentId and optional location path.

    Returns ``(parent_id, location_path)``:
    - ``parent_id``: the ``ItemId`` of the matched library (for the prefetch
      query's ``ParentId`` parameter), or ``None`` to fetch all items.
    - ``location_path``: the server-side path prefix for the matched location
      (e.g. ``"/classical"``), or ``None`` if no location scoping is needed.
      Used for post-prefetch filtering of items.
    """
    if not library_name and not location_name:
        return None, None

    libraries = client.discover_libraries()
    if not libraries:
        log.error("No music libraries found on server.")
        sys.exit(1)

    matched_location_path: str | None = None

    if library_name:
        match = [
            lib
            for lib in libraries
            if lib.get("Name", "").lower() == library_name.lower()
        ]
        if not match:
            names = ", ".join(lib.get("Name", "?") for lib in libraries)
            log.error(
                "Library '%s' not found. Available: %s", library_name, names
            )
            sys.exit(1)
        lib = match[0]
    else:
        # --location without --library: search all music libraries
        lib = None
        for candidate in libraries:
            for loc_path in candidate.get("Locations") or []:
                loc_leaf = loc_path.rstrip("/").rsplit("/", 1)[-1]
                if loc_leaf.lower() == location_name.lower():
                    lib = candidate
                    matched_location_path = loc_path
                    break
            if lib:
                break
        if not lib:
            all_locs = [
                loc_path.rstrip("/").rsplit("/", 1)[-1]
                for candidate in libraries
                for loc_path in candidate.get("Locations") or []
            ]
            log.error(
                "Location '%s' not found. Available: %s",
                location_name,
                ", ".join(all_locs),
            )
            sys.exit(1)

    parent_id = lib.get("ItemId", "")
    if not parent_id:
        log.error("Library '%s' has no ItemId.", lib.get("Name", "?"))
        sys.exit(1)

    if location_name and library_name:
        # Find the specific location within the matched library
        for loc_path in lib.get("Locations") or []:
            loc_leaf = loc_path.rstrip("/").rsplit("/", 1)[-1]
            if loc_leaf.lower() == location_name.lower():
                matched_location_path = loc_path
                log.info(
                    "Scoping to location '%s' in library '%s'",
                    location_name,
                    lib.get("Name"),
                )
                return parent_id, matched_location_path
        locs = [
            loc_path.rstrip("/").rsplit("/", 1)[-1]
            for loc_path in lib.get("Locations") or []
        ]
        log.error(
            "Location '%s' not found in library '%s'. Available: %s",
            location_name,
            lib.get("Name"),
            ", ".join(locs),
        )
        sys.exit(1)

    log.info("Scoping to library '%s' (ID: %s)", lib.get("Name"), parent_id)
    return parent_id, matched_location_path


def _filter_by_location(
    items: dict[str, dict], location_path: str
) -> dict[str, dict]:
    """Post-prefetch filter: keep only items whose server Path starts with the location path."""
    prefix = location_path.rstrip("/") + "/"
    filtered = {
        norm_path: item
        for norm_path, item in items.items()
        if (item.get("Path") or "").startswith(prefix)
    }
    log.info(
        "Location filter: %d / %d items under %s",
        len(filtered),
        len(items),
        location_path,
    )
    return filtered
```

- [ ] **Step 5: Wire scoping into `process_library()` and `force_rate_library()`**

At the top of both `process_library()` and `force_rate_library()`, after creating the client:

```python
    # Resolve library/location scope
    parent_id, location_path = _resolve_library_scope(
        client,
        config.library_name,
        config.location_name,
    )
```

Pass `parent_id` to `prefetch_audio_items()`:

```python
    server_items = client.prefetch_audio_items(
        include_media_sources=True, parent_id=parent_id
    )
```

Apply post-prefetch location filtering when `location_path` is set:

```python
    if location_path:
        server_items = _filter_by_location(server_items, location_path)
```

In `build_config`, set the `Config` fields:

```python
    library_name=getattr(args, "library", None),
    location_name=getattr(args, "location", None),
```

(Add these as keyword arguments to the `Config(...)` constructor call.)

- [ ] **Step 6: Make library_paths optional when --library/--location scoping is used**

When `--library` or `--location` is provided, `library_paths` should not be required. Adjust the `if not raw_paths` error in `build_config`:

```python
    has_scope = getattr(args, "library", None) or getattr(args, "location", None)
    if not raw_paths and getattr(args, "command", None) not in ("genres",) and not has_scope:
        print(
            "Error: library_path is required. Provide it via command-line argument, "
            "TAGLRC_LIBRARY_PATH environment variable, or [general].library_path in the TOML config.",
            file=sys.stderr,
        )
        sys.exit(1)
```

In `process_library` and `force_rate_library`, wrap the `_validate_library_paths` / `_is_under_roots` filtering in a conditional:

```python
    # Path-based scoping (only when library_paths are provided and no --library/--location)
    if config.library_paths and parent_id is None:
        _validate_library_paths(config.library_paths)
        lib_roots = [Path(_normalize_path(str(lp))) for lp in config.library_paths]
        items_in_scope = {
            path: item
            for path, item in server_items.items()
            if _is_under_roots(path, lib_roots)
        }
    else:
        items_in_scope = server_items
```

- [ ] **Step 6: Verify import and ruff**

---

### Task 12: UAT — Library/location scoping

- [ ] **Step 1: UAT `--library` flag**

```bash
../.venv/bin/python3 SetMusicParentalRating.py scan --library Music --dry-run -v 2>&1 | head -30
```

Expected: discovers libraries, scopes to "Music", logs "Scoping to library 'Music'".

- [ ] **Step 2: UAT `--location` flag**

```bash
../.venv/bin/python3 SetMusicParentalRating.py scan --location classical --dry-run -v 2>&1 | head -30
```

Expected: finds "classical" location, scopes accordingly.

- [ ] **Step 3: UAT unknown library/location errors**

```bash
../.venv/bin/python3 SetMusicParentalRating.py scan --library Nonexistent --dry-run 2>&1
../.venv/bin/python3 SetMusicParentalRating.py scan --location nonexistent --dry-run 2>&1
```

Expected: clear error messages listing available libraries/locations.

- [ ] **Step 4: UAT no library_path required with scoping**

```bash
../.venv/bin/python3 SetMusicParentalRating.py scan --library Music --dry-run -v 2>&1 | head -10
```

Expected: works without positional library_path arguments.

---

### Task 13: Commit and create PR C

- [ ] **Step 1: Commit**

```bash
git add SetMusicParentalRating/SetMusicParentalRating.py SetMusicParentalRating/explicit_config.example.toml
git commit -m "feat: add library/location discovery and scoping (#40)

Add discover_libraries() via GET /Library/VirtualFolders.
Add --library NAME and --location NAME flags to scope the prefetch
query using ParentId. library_path positional args are no longer
required when --library or --location provides scoping.

Closes #40"
```

- [ ] **Step 2: Create stacked PR via Graphite**

```bash
gt create -m "feat: add library/location discovery and scoping (#40)"
gt stack submit
```

---

## Chunk 4: PR D — Rename Subcommands (#41)

### Task 14: Rename subcommands

**Files:**
- Modify: `SetMusicParentalRating/SetMusicParentalRating.py` — `build_parser()`, `main()`, help text

- [ ] **Step 1: Rename `scan` → `rate` in `build_parser()`**

Rename the `scan` subparser:

```python
    rate_parser = subparsers.add_parser(
        "rate",
        parents=[shared],
        help="Fetch lyrics from server, detect explicit content, set ratings",
        description="Fetch lyrics from the media server API, detect explicit "
        "content, and set OfficialRating on matching tracks.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_RATE_EXAMPLES,
    )
```

Update `_RATE_EXAMPLES` (currently `_SCAN_EXAMPLES`) to use the new command name.

- [ ] **Step 2: Rename `rate` → `force` in `build_parser()`**

Rename the old `rate` subparser:

```python
    force_parser = subparsers.add_parser(
        "force",
        parents=[shared],
        help="Set a fixed rating on all tracks in scope (no lyrics evaluation)",
        description="Skip lyrics detection and set a fixed OfficialRating on ALL "
        "audio tracks in the configured scope.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_FORCE_EXAMPLES,
    )
```

Update the examples constant name and content.

- [ ] **Step 3: Add `reset` subparser**

Note: `--library`, `--location`, `--server`, `--server-url`, `--api-key`, and `-v` are already inherited from `parents=[shared]`. Only add subcommand-specific flags (`-n`, `--report`).

```python
    reset_parser = subparsers.add_parser(
        "reset",
        parents=[shared],
        help="Remove OfficialRating from all tracks in scope",
        description="Remove OfficialRating from ALL audio tracks in the "
        "configured scope. This is a destructive operation.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_RESET_EXAMPLES,
    )
    reset_parser.add_argument(
        "-n",
        "--dry-run",
        action="store_true",
        help="Analyze only — no server updates",
    )
    reset_parser.add_argument(
        "--report",
        default=None,
        help="CSV report output path",
    )
```

Add `_RESET_EXAMPLES`:

```python
_RESET_EXAMPLES = """\
examples:
  # Remove all ratings from the Music library
  %(prog)s --library Music

  # Dry run — show what would be cleared
  %(prog)s --library Music --dry-run
"""
```

- [ ] **Step 4: Add `reset_library()` function**

Insert after `force_rate_library`:

```python
def reset_library(config: Config) -> list[DetectionResult]:
    """'reset' subcommand: remove OfficialRating from all audio tracks in scope."""
    if not config.server_url or not config.server_api_key:
        log.error("'reset' requires a server URL and API key.")
        sys.exit(1)

    client = MediaServerClient(
        config.server_url, config.server_api_key, config.server_type
    )
    parent_id, location_path = _resolve_library_scope(
        client,
        config.library_name,
        config.location_name,
    )
    try:
        all_items = client.prefetch_audio_items(parent_id=parent_id)
    except MediaServerError as exc:
        log.error("Failed to prefetch server items: %s", exc)
        sys.exit(1)
    if location_path:
        all_items = _filter_by_location(all_items, location_path)

    results: list[DetectionResult] = []
    for norm_path, item in all_items.items():
        item_id, current, artist, album = _item_fields(item)
        dr = DetectionResult(
            sidecar_path=None,
            audio_path=Path(norm_path) if norm_path else None,
            tier=None,
            server_item_id=item_id,
            previous_rating=current,
            artist=artist,
            album=album,
            source="reset",
        )
        if not item_id:
            dr.action = "not_found_in_server"
        elif not current:
            dr.action = "skipped"
            log.debug("No rating to clear: %s", norm_path)
        elif config.dry_run:
            dr.action = "dry_run_clear"
            log.info("[DRY RUN] Would clear rating from %s (was %s)", norm_path, current)
        else:
            action = _apply_rating(client, item_id, "", norm_path)
            dr.action = "cleared" if action == "set" else action
        results.append(dr)

    for r in results:
        if not r.server_type:
            r.server_type = config.server_type
    return results
```

- [ ] **Step 5: Update `main()` to handle renamed commands**

Replace the server iteration loop body in `main()`. Instead of checking `config.force_rating`, dispatch by `args.command`:

```python
    for server in config.servers:
        srv_config = replace(
            config,
            server_url=server.url,
            server_api_key=server.api_key,
            server_type=server.server_type,
        )
        label = (
            f"{server.name} ({server.server_type.title()})" if multi else ""
        )
        if label:
            log.info("--- Processing %s ---", label)

        try:
            if args.command == "rate":
                results = process_library(srv_config)
            elif args.command == "force":
                results = force_rate_library(srv_config)
            elif args.command == "reset":
                results = reset_library(srv_config)
            else:
                results = []
        except SystemExit as exc:
            log.error("%s failed (exit %s).", label or "Server", exc.code)
            results = []

        all_results.extend(results)
        if multi:
            print_summary(results, label=label)
```

Also update the `DetectionResult.source` field comment to include `"reset"` as a valid value.

- [ ] **Step 6: Remove `genres` subparser**

Remove the `genres` subparser from `build_parser()` and the `list_genres_mode()` dispatch from `main()`. Keep the `list_genres_mode()` function and `list_genres()` method — they'll be used by `configure` in Milestone 3.

- [ ] **Step 7: Update help text and examples**

Update `_MAIN_EXAMPLES`:

```python
_MAIN_EXAMPLES = """\
subcommands:
  rate    Fetch lyrics from server, detect explicit content, set ratings
  force   Set a fixed rating on all tracks in scope (no lyrics evaluation)
  reset   Remove OfficialRating from all tracks in scope

examples:
  %(prog)s rate --dry-run --report report.csv
  %(prog)s rate --library Music --dry-run
  %(prog)s force G --library "Classical Music"
  %(prog)s reset --library Music --dry-run
"""
```

- [ ] **Step 8: Verify import and ruff**

---

### Task 15: UAT — Renamed subcommands

- [ ] **Step 1: UAT `rate` (was `scan`)**

```bash
../.venv/bin/python3 SetMusicParentalRating.py rate --dry-run -v 2>&1 | head -20
```

- [ ] **Step 2: UAT `force` (was `rate`)**

```bash
../.venv/bin/python3 SetMusicParentalRating.py force G --library Music --dry-run -v 2>&1 | head -20
```

- [ ] **Step 3: UAT `reset`**

```bash
../.venv/bin/python3 SetMusicParentalRating.py reset --library Music --dry-run -v 2>&1 | head -20
```

Expected: `[DRY RUN] Would clear rating from ...` for any tracks that have ratings.

- [ ] **Step 4: UAT old command names fail**

```bash
../.venv/bin/python3 SetMusicParentalRating.py scan --dry-run 2>&1
```

Expected: error — `scan` is not a valid subcommand.

---

### Task 16: Commit and create PR D

- [ ] **Step 1: Commit**

```bash
git add SetMusicParentalRating/SetMusicParentalRating.py
git commit -m "feat: rename subcommands and add reset (#41)

Rename scan→rate, rate→force. Add 'reset' subcommand to remove
OfficialRating from all tracks in scope. Remove standalone 'genres'
subcommand (will be folded into 'configure' in Milestone 3).

Closes #41"
```

- [ ] **Step 2: Stack via Graphite**

```bash
gt create -m "feat: rename subcommands and add reset (#41)"
gt stack submit
```

---

## Chunk 5: PR E — Overwrite/Skip-existing (#42)

### Task 17: Add `--overwrite` / `--skip-existing` flags

**Files:**
- Modify: `SetMusicParentalRating/SetMusicParentalRating.py`

- [ ] **Step 1: Add fields to `Config`**

```python
    overwrite: bool = True  # default: re-evaluate and overwrite
```

- [ ] **Step 2: Add CLI flags to `rate` and `force` subparsers**

Add to both `rate_parser` and `force_parser`:

```python
    parser.add_argument(
        "--overwrite",
        action="store_true",
        default=None,
        help="Re-evaluate and update tracks that already have a rating (default)",
    )
    parser.add_argument(
        "--skip-existing",
        action="store_true",
        default=None,
        help="Skip tracks that already have any rating",
    )
```

- [ ] **Step 3: Resolve overwrite in `build_config`**

After the existing config merge, add:

```python
    # Overwrite behavior: CLI > TOML > default (True)
    cli_overwrite = getattr(args, "overwrite", None)
    cli_skip = getattr(args, "skip_existing", None)
    if cli_overwrite and cli_skip:
        print(
            "Error: --overwrite and --skip-existing are mutually exclusive.",
            file=sys.stderr,
        )
        sys.exit(1)
    if cli_skip:
        overwrite = False
    elif cli_overwrite:
        overwrite = True
    else:
        overwrite = toml.get("general", {}).get("overwrite", True)
```

Pass `overwrite=overwrite` to `Config(...)`.

- [ ] **Step 4: Remove `--clear` flag from `rate` subparser**

Remove the `--clear` argument from the `rate_parser`. Remove `clear: bool = False` from `Config`.

- [ ] **Step 5: Update `process_library()` to respect `overwrite`**

In `process_library()`, find the lyrics evaluation block (currently around the `if tier is not None:` / `elif config.clear:` / `else:` chain). Replace it with:

```python
            if tier is not None:
                if not config.overwrite and prev_rating:
                    dr.action = "skipped"
                    log.debug("Skipping (has rating %s): %s", prev_rating, norm_path)
                else:
                    dr.action = _decide_rating_action(
                        client=client,
                        item_id=item_id,
                        tier=tier,
                        current_rating=prev_rating,
                        label=norm_path,
                        dry_run=config.dry_run,
                    )
            elif config.overwrite and prev_rating:
                # Lyrics are clean but track has a rating — clear it
                dr.action = _decide_clear_action(
                    client=client,
                    item_id=item_id,
                    current_rating=prev_rating,
                    label=norm_path,
                    dry_run=config.dry_run,
                )
            else:
                dr.action = "skipped"
```

This replaces the old `config.clear` check (the `elif config.clear:` branch). When `overwrite` is True (default), tracks with clean lyrics that have a rating get their rating cleared — the old `--clear` behavior is now the default.

Also update the **genre pass** in `process_library()` to respect `--skip-existing`. In the genre fallback block (the `if config.g_genres:` section), add a skip check before calling `_decide_rating_action`:

```python
        if config.g_genres:
            matched_genre = match_g_genre(item, config.g_genres)
            if matched_genre is not None:
                if not config.overwrite and prev_rating:
                    # --skip-existing: don't override with genre-based G
                    continue
                # ... existing genre rating logic ...
```

- [ ] **Step 6: Update `force_rate_library()` to respect `skip_existing`**

In `force_rate_library()`, add skip logic:

```python
        if not config.overwrite and current:
            dr.action = "skipped"
            log.debug("Skipping (has rating %s): %s", current, norm_path)
```

- [ ] **Step 7: Update TOML example with overwrite setting**

Add to `[general]` in `explicit_config.example.toml`:

```toml
# overwrite = true  # Re-evaluate tracks that already have a rating (default)
# overwrite = false  # Skip tracks that already have any rating (--skip-existing)
```

- [ ] **Step 8: Verify import and ruff**

---

### Task 18: UAT — Overwrite/skip-existing

- [ ] **Step 1: UAT default behavior (overwrite)**

```bash
../.venv/bin/python3 SetMusicParentalRating.py rate --dry-run -v 2>&1 | head -30
```

Expected: processes all tracks including those with existing ratings.

- [ ] **Step 2: UAT `--skip-existing`**

```bash
../.venv/bin/python3 SetMusicParentalRating.py rate --skip-existing --dry-run -v 2>&1 | grep -c "Skipping"
```

Expected: tracks with existing ratings show "Skipping".

- [ ] **Step 3: UAT `--overwrite` explicit**

```bash
../.venv/bin/python3 SetMusicParentalRating.py rate --overwrite --dry-run -v 2>&1 | head -20
```

Expected: same as default — processes all tracks.

- [ ] **Step 4: UAT mutual exclusion**

```bash
../.venv/bin/python3 SetMusicParentalRating.py rate --overwrite --skip-existing --dry-run 2>&1
```

Expected: "Error: --overwrite and --skip-existing are mutually exclusive."

---

### Task 19: Commit, create PR E, and update docs

- [ ] **Step 1: Commit**

```bash
git add SetMusicParentalRating/SetMusicParentalRating.py SetMusicParentalRating/explicit_config.example.toml
git commit -m "feat: add --overwrite/--skip-existing, remove --clear (#42)

Add --overwrite (default) and --skip-existing flags to rate and force.
--overwrite re-evaluates and updates all tracks, including clearing
ratings when lyrics are clean (subsuming old --clear behavior).
--skip-existing skips tracks that already have any rating.
Default is configurable via overwrite = true/false in TOML [general].
Remove --clear flag.

Closes #42"
```

- [ ] **Step 2: Stack via Graphite and submit**

```bash
gt create -m "feat: add --overwrite/--skip-existing, remove --clear (#42)"
gt stack submit
```

- [ ] **Step 3: Update CLAUDE.md**

Update the Architecture section of `CLAUDE.md` to reflect:
- New named server model (`[servers.*]` + `{LABEL}_API_KEY`)
- Auto-detection via `/System/Info/Public`
- `--server NAME`, `--library NAME`, `--location NAME` flags
- Renamed subcommands: `rate`, `force`, `reset`
- `--overwrite`/`--skip-existing` behavior
- Removed: `--server-type`, `--clear`, `genres` subcommand, legacy env vars

Commit separately:

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md for Milestone 2 changes"
```

---

## Execution Notes

### Server availability
- **Jellyfin** at localhost:8097 is confirmed running. Use for UAT.
- **Emby** at localhost:8096 may not be running. Start it if available; otherwise UAT against Jellyfin only and note in PR description.

### Ruff and import smoke test
Run before **every** commit:
```bash
ruff check . && ruff format . && cd SetMusicParentalRating && ../.venv/bin/python3 -c "import SetMusicParentalRating"
```

### PR merge order
1. **PR A** (#37 + #38) — merge first
2. **PR B** (#39) and **PR C** (#40) — can be merged in parallel after A
3. **PR D** (#41) — after C merges
4. **PR E** (#42) — after D merges

### Graphite stack structure
- PR A: standalone branch `feat/auto-detect-named-servers`
- Stack 1: PR B (off A)
- Stack 2: PR C → PR D → PR E (off A)

Use `gt stack restack` after PR A merges to rebase both stacks onto main.
