#!/usr/bin/env python3
"""SetMusicParentalRating — fetch lyrics from the Emby or Jellyfin API, detect
explicit content, and set OfficialRating on matching audio tracks.

Python 3.11+ recommended (uses tomllib from stdlib).
On older Python, falls back to the tomli package.
"""

from __future__ import annotations

__version__ = "1.0.0"

import argparse
import csv
import json
import logging
import os
import re
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass, field, replace
from pathlib import Path

try:
    import tomllib  # Python 3.11+
except ModuleNotFoundError:
    try:
        import tomli as tomllib  # type: ignore[no-redef]  # pip install tomli
    except ModuleNotFoundError:
        tomllib = None  # type: ignore[assignment]

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

DEFAULT_R_STEMS: list[str] = [
    "fuck",
    "shit",
    "pussy",
    "cock",
    "cum",
    "faggot",
]
DEFAULT_R_EXACT: list[str] = [
    "blowjob",
    "cocksucker",
    "motherfuck",
    "bullshit",
]
DEFAULT_PG13_STEMS: list[str] = [
    "bitch",
    "whore",
    "slut",
]
DEFAULT_PG13_EXACT: list[str] = [
    "hoe",
    "asshole",
    "piss",
]
DEFAULT_FALSE_POSITIVES: list[str] = [
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

log = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Exceptions
# ---------------------------------------------------------------------------


class MediaServerError(Exception):
    """Raised when a media server API call fails."""

    def __init__(self, message: str, status_code: int | None = None) -> None:
        super().__init__(message)
        self.status_code = status_code


# ---------------------------------------------------------------------------
# Dataclasses
# ---------------------------------------------------------------------------


@dataclass
class ServerConfig:
    """Named server configuration — one per [servers.*] TOML section."""

    name: str
    url: str
    api_key: str
    server_type: str = ""  # auto-detected via /System/Info/Public


@dataclass
class Config:
    library_paths: list[Path]
    server_url: str
    server_api_key: str
    server_type: str = "emby"
    r_stems: list[str] = field(default_factory=lambda: list(DEFAULT_R_STEMS))
    r_exact: list[str] = field(default_factory=lambda: list(DEFAULT_R_EXACT))
    pg13_stems: list[str] = field(default_factory=lambda: list(DEFAULT_PG13_STEMS))
    pg13_exact: list[str] = field(default_factory=lambda: list(DEFAULT_PG13_EXACT))
    false_positives: list[str] = field(
        default_factory=lambda: list(DEFAULT_FALSE_POSITIVES)
    )
    dry_run: bool = False
    overwrite: bool = True  # default: re-evaluate and overwrite
    force_rating: str | None = None
    report_path: Path | None = None
    g_genres: list[str] = field(default_factory=list)
    servers: list[ServerConfig] = field(default_factory=list)
    library_name: str | None = None
    location_name: str | None = None

    def __post_init__(self) -> None:
        if self.server_type not in ("emby", "jellyfin"):
            raise ValueError(
                f"server_type must be 'emby' or 'jellyfin', got {self.server_type!r}"
            )
        self._r_exact_patterns = _compile_exact_patterns(self.r_exact)
        self._pg13_exact_patterns = _compile_exact_patterns(self.pg13_exact)
        self.g_genres = [g.strip() for g in self.g_genres if g.strip()]


@dataclass
class DetectionResult:
    sidecar_path: Path | None
    audio_path: Path | None
    tier: str | None  # "R", "PG-13", "G" (genre-matched), or None (clean)
    matched_words: list[str] = field(default_factory=list)
    server_item_id: str | None = None
    action: str = (
        ""  # set | cleared | skipped | already_correct | not_found_in_server |
    )
    #                    server_unavailable | error | no_audio_file | dry_run | dry_run_clear |
    #                    g_genre | g_genre_already_correct | dry_run_g_genre
    previous_rating: str = ""
    artist: str = ""
    album: str = ""
    source: str = ""  # "lyrics" | "genre" | "force" | "reset"
    server_type: str = (
        ""  # "emby" | "jellyfin"; populated by process_library / force_rate_library
    )


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------


def load_env(path: Path, *, required: bool = False) -> dict[str, str]:
    """Parse a .env file into a dict. Skips comments and blank lines."""
    env: dict[str, str] = {}
    if not path.is_file():
        log.warning(".env file not found at %s", path)
        return env
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if "=" not in line:
                continue
            key, _, value = line.partition("=")
            key = key.strip()
            value = value.strip()
            # Strip surrounding quotes
            if len(value) >= 2 and value[0] == value[-1] and value[0] in ("'", '"'):
                value = value[1:-1]
            env[key] = value
    except OSError as exc:
        if required:
            print(f"Error: could not read env file: {path} ({exc})", file=sys.stderr)
            sys.exit(1)
        log.warning("Could not read .env file %s: %s", path, exc)
    return env


def load_toml_config(path: Path) -> dict:
    """Load TOML config, returning {} if the file is missing."""
    if not path.is_file():
        log.warning("Config file not found at %s — using defaults", path)
        return {}
    if tomllib is None:
        log.warning(
            "No TOML parser available (need Python 3.11+ or 'pip install tomli') — using defaults"
        )
        return {}
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as exc:
        log.warning("Could not parse config %s: %s — using defaults", path, exc)
        return {}


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
                product = data.get("ProductName")
                if product == "Jellyfin Server":
                    log.info("Auto-detected Jellyfin at %s", clean_url)
                    return "jellyfin"
                if product:
                    log.info(
                        "Auto-detected Emby at %s (ProductName=%s)", clean_url, product
                    )
                    return "emby"
                # ProductName missing/null — fall through to header check
            except (json.JSONDecodeError, AttributeError) as exc:
                log.warning(
                    "Could not parse JSON from %s (falling back to Server header): %s",
                    endpoint,
                    exc,
                )
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


def _resolve_servers(
    args: argparse.Namespace,
    toml: dict,
    env_file: dict[str, str],
) -> list[ServerConfig]:
    """Resolve server list from CLI overrides or TOML [servers.*] sections.

    Precedence:
    1. ``--server-url`` + ``--api-key`` → single one-off server
    2. TOML ``[servers.*]`` sections → named servers

    For each server without an explicit ``type``, auto-detects via
    ``/System/Info/Public``.
    """
    cli_server_url = (getattr(args, "server_url", None) or "").strip()
    cli_api_key = (getattr(args, "api_key", None) or "").strip()

    # --- 1. CLI one-off override ---
    if cli_server_url and cli_api_key:
        try:
            server_type = detect_server_type(cli_server_url)
        except MediaServerError as exc:
            print(
                f"Error: cannot auto-detect server type at {cli_server_url}: {exc}",
                file=sys.stderr,
            )
            sys.exit(1)
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
        # Apply --server filter early so we only resolve selected servers
        selected = getattr(args, "server", None) or []
        if selected:
            unknown = [s for s in selected if s not in toml_servers]
            if unknown:
                avail = ", ".join(sorted(toml_servers.keys()))
                print(
                    f"Error: unknown server(s): {', '.join(unknown)}. "
                    f"Available: {avail}",
                    file=sys.stderr,
                )
                sys.exit(1)
            entries = {k: v for k, v in toml_servers.items() if k in selected}
        else:
            entries = {k: v for k, v in toml_servers.items() if isinstance(v, dict)}
        servers: list[ServerConfig] = []
        for name, srv_conf in entries.items():
            if not isinstance(srv_conf, dict):
                print(
                    f"Error: server '{name}' in config must be a table/section "
                    f"(got {type(srv_conf).__name__}). "
                    f'Use [servers.{name}] with url = "..." instead.',
                    file=sys.stderr,
                )
                sys.exit(1)
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
                os.environ.get(env_key_name, "") or env_file.get(env_key_name, "")
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
                try:
                    server_type = detect_server_type(url)
                except MediaServerError as exc:
                    print(
                        f"Error: cannot auto-detect server type for '{name}' "
                        f"at {url}: {exc}",
                        file=sys.stderr,
                    )
                    sys.exit(1)
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

    # --- 3. No servers configured ---
    print(
        "Error: no servers configured. Add [servers.*] sections to the TOML "
        "config, or use --server-url and --api-key for a one-off run.",
        file=sys.stderr,
    )
    sys.exit(1)


def build_config(args: argparse.Namespace) -> Config:
    """Merge config layers with precedence: CLI > os.environ > .env > TOML > defaults."""
    script_dir = Path(__file__).resolve().parent
    toml_path = (
        Path(args.config) if args.config else script_dir / "explicit_config.toml"
    )
    toml = load_toml_config(toml_path)

    if args.env_file:
        env_path = Path(args.env_file).expanduser()
        if not env_path.is_file():
            print(
                f"Error: specified env file does not exist or is not a regular file: {env_path}",
                file=sys.stderr,
            )
            sys.exit(1)
    else:
        env_path = script_dir.parent / ".env"
    env_file = load_env(env_path, required=bool(args.env_file))

    # --- library_paths (multi-path support) ---
    # CLI provides a list (nargs="*"); env var and TOML provide a single string or list.
    raw_paths: list[str] = []
    cli_paths = getattr(args, "library_path", None)
    if cli_paths:
        raw_paths = cli_paths
    else:
        env_lp = os.environ.get("TAGLRC_LIBRARY_PATH") or env_file.get(
            "TAGLRC_LIBRARY_PATH"
        )
        if env_lp:
            raw_paths = [env_lp]
        else:
            toml_lp = toml.get("general", {}).get("library_path")
            if isinstance(toml_lp, list):
                raw_paths = [p for p in toml_lp if p]
            elif toml_lp:
                raw_paths = [toml_lp]

    has_scope = getattr(args, "library", None) or getattr(args, "location", None)
    if (
        not raw_paths
        and getattr(args, "command", None) not in ("reset",)
        and not has_scope
    ):
        print(
            "Error: library_path is required. Provide it via command-line argument, "
            "TAGLRC_LIBRARY_PATH environment variable, or [general].library_path in the TOML config.",
            file=sys.stderr,
        )
        sys.exit(1)
    try:
        library_paths = [Path(p).expanduser() for p in raw_paths] if raw_paths else []
    except (RuntimeError, TypeError) as exc:
        print(
            f"Error: cannot expand library_path: {exc}",
            file=sys.stderr,
        )
        sys.exit(1)

    # --- Resolve servers (--server filtering applied inside) ---
    servers = _resolve_servers(args, toml, env_file)

    # Set server_url/api_key/type from first resolved server
    # _resolve_servers always returns non-empty or calls sys.exit
    active = servers[0]

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
        raw_overwrite = toml.get("general", {}).get("overwrite", True)
        if not isinstance(raw_overwrite, bool):
            print(
                f"Error: [general].overwrite must be true or false, "
                f"got {raw_overwrite!r}",
                file=sys.stderr,
            )
            sys.exit(1)
        overwrite = raw_overwrite

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
        overwrite=overwrite,
        force_rating=getattr(args, "rating", None),
        report_path=report_path,
        g_genres=g_genres,
        servers=servers,
        library_name=getattr(args, "library", None),
        location_name=getattr(args, "location", None),
    )


# ---------------------------------------------------------------------------
# LRC Parsing
# ---------------------------------------------------------------------------

_LRC_TIMESTAMP_RE = re.compile(r"\[\d{1,3}:\d{2}(?:\.\d{1,3})?\]")
_LRC_METADATA_RE = re.compile(r"^\[[a-z]{2,}:.*\]$", re.IGNORECASE | re.MULTILINE)


def strip_lrc_tags(text: str) -> str:
    """Remove LRC timestamp tags and metadata lines."""
    text = _LRC_TIMESTAMP_RE.sub("", text)
    text = _LRC_METADATA_RE.sub("", text)
    return text


def extract_embedded_lyrics(item: dict) -> str:
    """Extract embedded lyrics text from a media server item's MediaSources.

    Looks for internal Subtitle streams (IsExternal=False, Type='Subtitle')
    and returns their Extradata joined with newlines, stripped of LRC tags.
    Returns "" if no embedded lyrics are found.
    """
    fragments: list[str] = []
    for source in item.get("MediaSources") or []:
        for stream in source.get("MediaStreams") or []:
            if stream.get("IsExternal", True):
                continue
            if stream.get("Type") != "Subtitle":
                continue
            extradata = stream.get("Extradata")
            if isinstance(extradata, str) and extradata.strip():
                fragments.append(extradata)
            elif extradata is not None:
                log.debug(
                    "Unexpected Extradata type %s in MediaStream (track: %s); skipping",
                    type(extradata).__name__,
                    item.get("Path", "<unknown>"),
                )
    if not fragments:
        return ""
    return strip_lrc_tags("\n".join(fragments))


# ---------------------------------------------------------------------------
# Explicit Detection
# ---------------------------------------------------------------------------


def detect_stems(
    word_tokens: list[str],
    stems: list[str],
    false_positives: list[str],
) -> list[str]:
    """Substring match each stem against word tokens, filtered by false positives."""
    matched: list[str] = []
    fp_lower = [fp.lower() for fp in false_positives]
    for stem in stems:
        stem_l = stem.lower()
        for word in word_tokens:
            if stem_l in word:
                # Check if the matched word is (or contains) a false positive
                is_fp = any(fp in word for fp in fp_lower)
                if not is_fp:
                    matched.append(word)
                    break  # one match per stem is enough
    return matched


def _compile_exact_patterns(words: list[str]) -> list[tuple[str, re.Pattern[str]]]:
    """Precompile word-boundary regexes for exact matching."""
    return [(w, re.compile(r"\b" + re.escape(w) + r"\b", re.IGNORECASE)) for w in words]


def detect_exact(text: str, patterns: list[tuple[str, re.Pattern[str]]]) -> list[str]:
    """Word-boundary regex match using precompiled patterns."""
    return [word for word, pat in patterns if pat.search(text)]


def classify_lyrics(text: str, config: Config) -> tuple[str | None, list[str]]:
    """Classify lyrics text. Returns (tier, matched_words)."""
    if not text.strip():
        return None, []

    word_tokens = re.findall(r"[a-z']+", text.lower())

    # Check R tier first
    r_stem_hits = detect_stems(word_tokens, config.r_stems, config.false_positives)
    r_exact_hits = detect_exact(text, config._r_exact_patterns)
    if r_stem_hits or r_exact_hits:
        return "R", list(dict.fromkeys(r_stem_hits + r_exact_hits))

    # Then PG-13
    pg13_stem_hits = detect_stems(
        word_tokens, config.pg13_stems, config.false_positives
    )
    pg13_exact_hits = detect_exact(text, config._pg13_exact_patterns)
    if pg13_stem_hits or pg13_exact_hits:
        return "PG-13", list(dict.fromkeys(pg13_stem_hits + pg13_exact_hits))

    return None, []


def match_g_genre(item: dict, g_genres: list[str]) -> str | None:
    """Return the first Genres entry matching the safe list (case-insensitive), or None."""
    if not g_genres:
        return None
    lowered = {g.lower() for g in g_genres}
    for entry in item.get("Genres") or []:
        if not isinstance(entry, str):
            log.debug(
                "match_g_genre: unexpected non-string Genres entry %r in item %s",
                entry,
                item.get("Id", "<unknown>"),
            )
            continue
        if entry.lower() in lowered:
            return entry
    return None


# ---------------------------------------------------------------------------
# Emby API Client
# ---------------------------------------------------------------------------


class MediaServerClient:
    """Minimal Emby/Jellyfin HTTP client using urllib (stdlib)."""

    def __init__(self, base_url: str, api_key: str, server_type: str = "emby") -> None:
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.server_type = server_type
        self._user_id: str | None = None

    def _request(
        self,
        method: str,
        path: str,
        body: dict | None = None,
    ) -> dict | None:
        url = f"{self.base_url}{path}"
        data = json.dumps(body).encode("utf-8") if body is not None else None
        req = urllib.request.Request(url, data=data, method=method)
        if self.server_type == "jellyfin":
            req.add_header("X-MediaBrowser-Token", self.api_key)
        else:
            req.add_header("X-Emby-Token", self.api_key)
        req.add_header("Content-Type", "application/json")
        req.add_header("Accept", "application/json")
        log.debug("%s %s %s", self.server_type.title(), method, url)
        try:
            with urllib.request.urlopen(req, timeout=15) as resp:
                resp_data = resp.read()
                if resp_data:
                    try:
                        return json.loads(resp_data)
                    except json.JSONDecodeError as exc:
                        raise MediaServerError(
                            f"Non-JSON response on {method} {path}: {resp_data[:200]!r}"
                        ) from exc
                return None
        except urllib.error.HTTPError as exc:
            body_snippet = ""
            try:
                body_snippet = exc.read().decode("utf-8", errors="replace")[:1024]
            except Exception as inner_exc:
                log.debug("Could not read HTTP error body: %s", inner_exc)
            raise MediaServerError(
                f"HTTP {exc.code} on {method} {path}: {body_snippet}",
                status_code=exc.code,
            ) from exc
        except urllib.error.URLError as exc:
            raise MediaServerError(
                f"Connection error on {method} {path}: {exc.reason}"
            ) from exc

    def _request_text(self, method: str, path: str) -> str:
        """Like _request(), but returns the raw response body as text.

        Used for endpoints that return plain text instead of JSON (e.g. the
        subtitle stream endpoint).  Raises MediaServerError on HTTP/connection
        errors, just like _request().
        """
        url = f"{self.base_url}{path}"
        req = urllib.request.Request(url, method=method)
        if self.server_type == "jellyfin":
            req.add_header("X-MediaBrowser-Token", self.api_key)
        else:
            req.add_header("X-Emby-Token", self.api_key)
        log.debug("%s %s %s (text)", self.server_type.title(), method, url)
        try:
            with urllib.request.urlopen(req, timeout=15) as resp:
                return resp.read().decode("utf-8", errors="replace")
        except urllib.error.HTTPError as exc:
            body_snippet = ""
            try:
                body_snippet = exc.read().decode("utf-8", errors="replace")[:1024]
            except Exception as inner_exc:
                log.debug("Could not read HTTP error body: %s", inner_exc)
            raise MediaServerError(
                f"HTTP {exc.code} on {method} {path}: {body_snippet}",
                status_code=exc.code,
            ) from exc
        except urllib.error.URLError as exc:
            raise MediaServerError(
                f"Connection error on {method} {path}: {exc.reason}"
            ) from exc

    def prefetch_audio_items(
        self, *, include_media_sources: bool = False, parent_id: str | None = None
    ) -> dict[str, dict]:
        """Paginated fetch of all Audio items. Returns {normalized_path: item}.

        Pass include_media_sources=True to append MediaSources to the Fields
        parameter on Emby (includes embedded lyrics in MediaStreams.Extradata at
        the cost of a larger payload). On Jellyfin this flag has no effect —
        embedded lyrics are fetched per-track via /Audio/{id}/Lyrics by the caller.

        Pass parent_id to scope the query to a specific library (ItemId from
        /Library/VirtualFolders).
        """
        fields = "Path,OfficialRating,AlbumArtist,Album,Genres"
        if include_media_sources and self.server_type == "emby":
            fields += ",MediaSources"
        items_by_path: dict[str, dict] = {}
        start_index = 0
        page_size = 500
        total = 0
        uid = self._get_user_id()
        while True:
            parent_filter = f"&ParentId={parent_id}" if parent_id else ""
            result = self._request(
                "GET",
                f"/Users/{uid}/Items?Recursive=true&IncludeItemTypes=Audio"
                f"&Fields={fields}{parent_filter}"
                f"&StartIndex={start_index}&Limit={page_size}",
            )
            if not result:
                if items_by_path:
                    log.warning(
                        "Server returned empty body mid-pagination after %d items "
                        "(expected %d); prefetch will be incomplete",
                        len(items_by_path),
                        total,
                    )
                break
            batch = result.get("Items", [])
            if not batch:
                break
            for item in batch:
                p = item.get("Path", "")
                if p:
                    items_by_path[_normalize_path(p)] = item
            total = result.get("TotalRecordCount", 0)
            start_index += len(batch)
            log.debug("Fetched %d / %d audio items", start_index, total)
            if start_index >= total:
                break
        log.info("Prefetched %d audio items from server", len(items_by_path))
        return items_by_path

    def _get_user_id(self) -> str:
        """Fetch and cache the first user ID (needed for user-scoped endpoints)."""
        if self._user_id is None:
            users = self._request("GET", "/Users")
            if not users:
                raise MediaServerError("No users returned from /Users")
            user_id = users[0].get("Id")
            if not user_id:
                raise MediaServerError("First user has no 'Id' field")
            self._user_id = user_id
            log.debug("Using server user ID: %s", self._user_id)
        return self._user_id

    def get_item(self, item_id: str) -> dict:
        """GET /Users/{userId}/Items/{id} — full item for round-trip update."""
        uid = self._get_user_id()
        result = self._request("GET", f"/Users/{uid}/Items/{item_id}")
        if result is None:
            raise MediaServerError(
                f"Empty response for GET /Users/{uid}/Items/{item_id}"
            )
        return result

    def fetch_lyrics_jellyfin(self, item_id: str) -> str:
        """Fetch embedded lyrics for a Jellyfin track via GET /Audio/{item_id}/Lyrics.

        Returns plain lyric text (LRC timestamps stripped), or "" if the item
        has no lyrics or a non-auth request fails. Raises MediaServerError on
        401/403 (auth/permission failures). Requires no lyrics plugin.
        """
        try:
            data = self._request("GET", f"/Audio/{item_id}/Lyrics")
        except MediaServerError as exc:
            if exc.status_code in (401, 403):
                raise  # auth/permission — don't mask
            if exc.status_code == 404:
                return ""
            log.warning("Jellyfin lyrics fetch failed for item %s: %s", item_id, exc)
            return ""
        if not data:
            return ""
        if not isinstance(data, dict):
            log.warning(
                "Jellyfin lyrics endpoint returned unexpected type %s for item %s",
                type(data).__name__,
                item_id,
            )
            return ""
        raw_lyrics = data.get("Lyrics") or []
        if not isinstance(raw_lyrics, list):
            log.warning(
                "Jellyfin lyrics payload has unexpected Lyrics type for item %s",
                item_id,
            )
            return ""
        lines = [
            entry.get("Text", "")
            for entry in raw_lyrics
            if isinstance(entry, dict) and entry.get("Text")
        ]
        return strip_lrc_tags("\n".join(lines))

    def fetch_lyrics_emby(self, item: dict) -> str:
        """Fetch lyrics for an Emby audio item via the subtitle stream endpoint.

        Checks the item's ``MediaSources[].MediaStreams[]`` for external subtitle
        streams (``Type="Subtitle"``, ``Codec="lrc"``, ``IsExternal=True``) and
        fetches the lyrics text via
        ``GET /Videos/{itemId}/{mediaSourceId}/Subtitles/{streamIndex}/Stream.txt``.

        If no external subtitle stream is found, falls back to embedded lyrics
        extracted from ``Extradata`` on internal subtitle streams.

        Returns plain lyric text (LRC timestamps stripped), or ``""`` if no
        lyrics are found.
        """
        item_id = item.get("Id", "")
        item_path = item.get("Path", "<unknown>")
        if not item_id:
            log.warning("Item missing 'Id' for %s; cannot fetch lyrics", item_path)
            return ""

        # --- Try external subtitle streams first ---
        for source in item.get("MediaSources") or []:
            media_source_id = source.get("Id", "")
            if not media_source_id:
                log.debug("MediaSource missing 'Id' for %s; skipping source", item_path)
                continue
            for stream in source.get("MediaStreams") or []:
                if stream.get("Type") != "Subtitle":
                    continue
                if not stream.get("IsExternal", False):
                    continue
                if str(stream.get("Codec") or "").lower() != "lrc":
                    continue
                stream_index = stream.get("Index")
                if stream_index is None:
                    log.debug(
                        "External subtitle stream missing Index for %s; skipping",
                        item_path,
                    )
                    continue
                path = (
                    f"/Videos/{item_id}/{media_source_id}"
                    f"/Subtitles/{stream_index}/Stream.txt"
                )
                try:
                    text = self._request_text("GET", path)
                except MediaServerError as exc:
                    if exc.status_code in (401, 403):
                        raise  # auth/permission — don't mask
                    log.warning(
                        "Emby subtitle fetch failed for %s (stream %s): %s",
                        item_path,
                        stream_index,
                        exc,
                    )
                    continue
                cleaned = strip_lrc_tags(text)
                if cleaned.strip():
                    return cleaned
                log.debug(
                    "External subtitle stream for %s (stream %s) returned "
                    "no usable text after stripping LRC tags; trying next stream",
                    item_path,
                    stream_index,
                )

        # --- Fallback: embedded lyrics from Extradata ---
        return extract_embedded_lyrics(item)

    def fetch_lyrics(self, item: dict) -> str | None:
        """Fetch lyrics for an audio item, abstracting server-specific logic.

        Returns plain text lyrics (normalized via strip_lrc_tags) or None if
        the track has no lyrics.
        """
        if self.server_type == "emby":
            text = self.fetch_lyrics_emby(item)
        elif self.server_type == "jellyfin":
            item_id = item.get("Id", "")
            text = self.fetch_lyrics_jellyfin(item_id) if item_id else ""
        else:
            log.warning("fetch_lyrics: unsupported server_type %r", self.server_type)
            return None
        return text if text and text.strip() else None

    def update_item(self, item_id: str, item_body: dict) -> None:
        """POST /Items/{id} — send full item body with modified fields."""
        self._request("POST", f"/Items/{item_id}", body=item_body)
        log.debug("Updated item %s", item_id)

    def list_genres(self) -> list[str]:
        """Return sorted list of all Audio genre names via GET /MusicGenres?Recursive=true."""
        result = self._request(
            "GET",
            "/MusicGenres?Recursive=true",
        )
        if result is None:
            log.warning("list_genres: server returned an empty response body")
            return []
        if not isinstance(result, dict):
            raise MediaServerError(
                f"list_genres: unexpected response type {type(result).__name__!r}; "
                "expected a JSON object"
            )
        items = result.get("Items")
        if items is None:
            log.warning(
                "list_genres: server response missing 'Items' key; keys present: %s",
                list(result.keys()),
            )
            return []
        if not isinstance(items, list):
            raise MediaServerError(
                f"list_genres: 'Items' field is {type(items).__name__!r}, expected a list"
            )
        non_dict = sum(1 for item in items if not isinstance(item, dict))
        if non_dict:
            log.warning(
                "list_genres: skipped %d non-dict item(s) in Items list", non_dict
            )
        names = [
            item.get("Name", "")
            for item in items
            if isinstance(item, dict) and item.get("Name")
        ]
        return sorted(names, key=str.casefold)

    def discover_libraries(self) -> list[dict]:
        """GET /Library/VirtualFolders — return music libraries.

        Each entry has ``Name``, ``ItemId``, ``Locations`` (list of path
        strings), and ``CollectionType``.  Only libraries with
        ``CollectionType == "music"`` are returned.
        """
        result = self._request("GET", "/Library/VirtualFolders")
        if result is None:
            log.warning("discover_libraries: server returned empty body")
            return []
        if not isinstance(result, list):
            log.warning(
                "discover_libraries: unexpected response type: %r", type(result)
            )
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


def _normalize_path(p: str) -> str:
    """Normalize a path for dict lookup, handling mixed separators."""
    return os.path.normcase(os.path.normpath(p)).replace("\\", "/")


def _item_fields(item: dict) -> tuple[str | None, str, str, str]:
    """Extract (item_id, previous_rating, artist, album) from a server item."""
    return (
        item.get("Id"),
        item.get("OfficialRating", "") or "",
        item.get("AlbumArtist", "") or "",
        item.get("Album", "") or "",
    )


# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------


def _path_parts(
    audio_path: Path | None, library_paths: list[Path] | None
) -> tuple[str, str]:
    """Best-effort fallback using the path relative to a library root."""
    if audio_path is None:
        return "", ""
    parts = audio_path.parts
    if library_paths:
        for lp in library_paths:
            try:
                parts = audio_path.relative_to(lp).parts
                break
            except ValueError:
                continue
    # Artist/Album/Track (3+ segments) → (artist, album)
    if len(parts) >= 3:
        return parts[-3], parts[-2]
    # Album/Track (2 segments) → (empty, album)
    if len(parts) == 2:
        return "", parts[-2]
    return "", ""


def write_report(
    results: list[DetectionResult],
    path: Path,
    library_paths: list[Path] | None = None,
) -> None:
    """Write detection results to a CSV file."""
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "w", newline="", encoding="utf-8") as f:
            writer = csv.writer(f)
            writer.writerow(
                [
                    "artist",
                    "album",
                    "track",
                    "sidecar",
                    "tier",
                    "matched_words",
                    "previous_rating",
                    "action",
                    "source",
                    "server",
                ]
            )
            for r in results:
                path_artist, path_album = _path_parts(r.audio_path, library_paths)
                artist = r.artist or path_artist
                album = r.album or path_album
                track = (r.audio_path or r.sidecar_path or Path()).name
                writer.writerow(
                    [
                        artist,
                        album,
                        track,
                        str(r.sidecar_path) if r.sidecar_path else "",
                        r.tier or "",
                        "; ".join(r.matched_words),
                        r.previous_rating,
                        r.action,
                        r.source,
                        r.server_type,
                    ]
                )
    except OSError as exc:
        log.error("Cannot write report to %s: %s", path, exc)
        return
    log.info("Report written to %s", path)


# ---------------------------------------------------------------------------
# Orchestration
# ---------------------------------------------------------------------------


def _validate_library_paths(paths: list[Path]) -> None:
    """Validate that every library path is absolute, exists, and is a directory."""
    for lp in paths:
        if not lp.is_absolute():
            log.error("library_path must be an absolute path; got %r", str(lp))
            sys.exit(1)
        if not lp.exists():
            log.error("library_path does not exist: %s", lp)
            sys.exit(1)
        if not lp.is_dir():
            log.error("library_path is not a directory: %s", lp)
            sys.exit(1)


def _is_under_roots(norm_path: str, lib_roots: list[Path]) -> bool:
    """Return True if *norm_path* falls under any of the library roots."""
    p = Path(norm_path)
    return any(p.is_relative_to(r) for r in lib_roots)


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
            log.error("Library '%s' not found. Available: %s", library_name, names)
            sys.exit(1)
        lib = match[0]
    else:
        # --location without --library: search all music libraries
        lib = None
        for candidate in libraries:
            for loc_path in candidate.get("Locations") or []:
                loc_leaf = loc_path.rstrip("/\\").replace("\\", "/").rsplit("/", 1)[-1]
                if loc_leaf.lower() == location_name.lower():
                    lib = candidate
                    matched_location_path = loc_path
                    break
            if lib:
                break
        if not lib:
            all_locs = [
                loc_path.rstrip("/\\").replace("\\", "/").rsplit("/", 1)[-1]
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
            loc_leaf = loc_path.rstrip("/\\").replace("\\", "/").rsplit("/", 1)[-1]
            if loc_leaf.lower() == location_name.lower():
                matched_location_path = loc_path
                log.info(
                    "Scoping to location '%s' in library '%s'",
                    location_name,
                    lib.get("Name"),
                )
                return parent_id, matched_location_path
        locs = [
            loc_path.rstrip("/\\").replace("\\", "/").rsplit("/", 1)[-1]
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


def _filter_by_location(items: dict[str, dict], location_path: str) -> dict[str, dict]:
    """Post-prefetch filter: keep only items whose normalized path starts with the location path."""
    prefix = _normalize_path(location_path.rstrip("/\\")) + "/"
    filtered = {
        norm_path: item
        for norm_path, item in items.items()
        if norm_path.startswith(prefix)
    }
    log.info(
        "Location filter: %d / %d items under %s",
        len(filtered),
        len(items),
        location_path,
    )
    return filtered


def process_library(config: Config) -> list[DetectionResult]:
    """Fetch lyrics via API, classify, and set ratings.

    Single-pass architecture: iterates over all prefetched audio items,
    fetches lyrics via the unified ``fetch_lyrics()`` method, classifies,
    and decides rating actions. Items without lyrics fall through to the
    genre allow-list check when ``config.g_genres`` is configured.
    """
    if not config.server_url or not config.server_api_key:
        log.error(
            "'rate' requires a server URL and API key. "
            "Add [servers.*] sections to the TOML config, "
            "or use --server-url and --api-key for a one-off run."
        )
        sys.exit(1)

    client = MediaServerClient(
        config.server_url, config.server_api_key, config.server_type
    )

    # Resolve library/location scope
    parent_id, location_path = _resolve_library_scope(
        client,
        config.library_name,
        config.location_name,
    )

    try:
        server_items = client.prefetch_audio_items(
            include_media_sources=True, parent_id=parent_id
        )
    except MediaServerError as exc:
        log.error("Failed to prefetch server items: %s", exc)
        sys.exit(1)

    if location_path:
        server_items = _filter_by_location(server_items, location_path)

    # Path-based scoping (only when library_paths are provided and no --library/--location)
    if config.library_paths and parent_id is None:
        _validate_library_paths(config.library_paths)
        lib_roots = [Path(_normalize_path(str(lp))) for lp in config.library_paths]
        items_in_scope = {
            path: item
            for path, item in server_items.items()
            if _is_under_roots(path, lib_roots)
        }
        paths_display = ", ".join(str(lp) for lp in config.library_paths)
        log.info(
            "Scanning %d items under %s (of %d total)",
            len(items_in_scope),
            paths_display,
            len(server_items),
        )
    else:
        items_in_scope = server_items
        log.info("Scanning all %d items (no library path filter)", len(server_items))

    results: list[DetectionResult] = []

    for norm_path, item in items_in_scope.items():
        item_id, prev_rating, artist, album = _item_fields(item)
        if not item_id:
            log.warning("Server item at %s has no 'Id'; skipping", norm_path)
            continue

        # Try lyrics
        try:
            lyrics_text = client.fetch_lyrics(item)
        except MediaServerError as exc:
            if exc.status_code in (401, 403):
                log.error("Auth/permission error fetching lyrics: %s", exc)
                sys.exit(1)
            log.warning("Failed to fetch lyrics for %s: %s", norm_path, exc)
            lyrics_text = None

        if lyrics_text is not None:
            tier, matched = classify_lyrics(lyrics_text, config)
            dr = DetectionResult(
                sidecar_path=None,
                audio_path=Path(norm_path) if norm_path else None,
                tier=tier,
                matched_words=matched,
                server_item_id=item_id,
                previous_rating=prev_rating,
                artist=artist,
                album=album,
                source="lyrics",
            )
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
            results.append(dr)
            continue

        # No lyrics — try genre fallback
        if config.g_genres:
            matched_genre = match_g_genre(item, config.g_genres)
            if matched_genre is not None:
                if not config.overwrite and prev_rating:
                    dr = DetectionResult(
                        sidecar_path=None,
                        audio_path=Path(norm_path) if norm_path else None,
                        tier="G",
                        matched_words=[matched_genre],
                        server_item_id=item_id,
                        previous_rating=prev_rating,
                        artist=artist,
                        album=album,
                        source="genre",
                    )
                    dr.action = "skipped"
                    log.debug(
                        "Skipping genre match (has rating %s): %s",
                        prev_rating,
                        norm_path,
                    )
                    results.append(dr)
                    continue
                dr = DetectionResult(
                    sidecar_path=None,
                    audio_path=Path(norm_path) if norm_path else None,
                    tier="G",
                    matched_words=[matched_genre],
                    server_item_id=item_id,
                    previous_rating=prev_rating,
                    artist=artist,
                    album=album,
                    source="genre",
                )
                action = _decide_rating_action(
                    client=client,
                    item_id=item_id,
                    tier="G",
                    current_rating=prev_rating,
                    label=f"{norm_path} (genre: {matched_genre})",
                    dry_run=config.dry_run,
                    action_dry="dry_run_g_genre",
                    action_already="g_genre_already_correct",
                )
                if action == "set":
                    action = "g_genre"
                dr.action = action
                results.append(dr)

    for r in results:
        if not r.server_type:
            r.server_type = config.server_type
    return results


def force_rate_library(config: Config) -> list[DetectionResult]:
    """'force' subcommand: set a fixed rating on ALL audio tracks in scope."""
    if not config.server_url or not config.server_api_key:
        log.error(
            "'force' requires a server URL and API key. "
            "Add [servers.*] sections to the TOML config, "
            "or use --server-url and --api-key for a one-off run."
        )
        sys.exit(1)

    target = config.force_rating
    client = MediaServerClient(
        config.server_url, config.server_api_key, config.server_type
    )

    # Resolve library/location scope
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

    # Path-based scoping (only when library_paths are provided and no --library/--location)
    if config.library_paths and parent_id is None:
        _validate_library_paths(config.library_paths)
        lib_roots = [Path(_normalize_path(str(lp))) for lp in config.library_paths]
        items_in_scope = {
            path: item
            for path, item in all_items.items()
            if _is_under_roots(path, lib_roots)
        }
        paths_display = ", ".join(str(lp) for lp in config.library_paths)
        log.info(
            "Force-rating: %d items under %s (of %d total)",
            len(items_in_scope),
            paths_display,
            len(all_items),
        )
    else:
        items_in_scope = all_items
        log.info("Force-rating all %d items (no library path filter)", len(all_items))

    results: list[DetectionResult] = []
    for norm_path, item in items_in_scope.items():
        item_id, current, artist, album = _item_fields(item)
        dr = DetectionResult(
            sidecar_path=None,
            audio_path=Path(norm_path),
            tier=target,
            server_item_id=item_id,
            previous_rating=current,
            artist=artist,
            album=album,
            source="force",
        )
        if not item_id:
            dr.action = "not_found_in_server"
            log.warning(
                "Force-rating: server item at %s has no 'Id'; skipping", norm_path
            )
        elif not config.overwrite and current:
            dr.action = "skipped"
            log.debug("Skipping (has rating %s): %s", current, norm_path)
        elif current == target:
            dr.action = "already_correct"
            log.debug("Already %s: %s", target, norm_path)
        elif config.dry_run:
            dr.action = "dry_run"
            log.info("[DRY RUN] Would set %s on %s", target, norm_path)
        else:
            dr.action = _apply_rating(client, item_id, target, norm_path)
        results.append(dr)

    for r in results:
        if not r.server_type:
            r.server_type = config.server_type
    return results


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
            log.warning("Reset: server item at %s has no 'Id'; skipping", norm_path)
        elif not current:
            dr.action = "skipped"
            log.debug("No rating to clear: %s", norm_path)
        elif config.dry_run:
            dr.action = "dry_run_clear"
            log.info(
                "[DRY RUN] Would clear rating from %s (was %s)", norm_path, current
            )
        else:
            action = _apply_rating(client, item_id, "", norm_path)
            dr.action = "cleared" if action == "set" else action
        results.append(dr)

    for r in results:
        if not r.server_type:
            r.server_type = config.server_type
    return results


def list_genres_mode(config: Config) -> None:
    """'genres' subcommand: print all Audio genre names from the server to stdout. Exits with non-zero status on error."""
    if not config.server_url or not config.server_api_key:
        print(
            "Error: 'genres' requires a server URL and API key. "
            "Add [servers.*] sections to the TOML config, "
            "or use --server-url and --api-key for a one-off run.",
            file=sys.stderr,
        )
        sys.exit(1)
    client = MediaServerClient(
        config.server_url, config.server_api_key, config.server_type
    )
    try:
        genres = client.list_genres()
    except MediaServerError as exc:
        log.error("Failed to retrieve genres from server: %s", exc)
        sys.exit(1)
    print("=== Audio Genres ===")
    for g in genres:
        print(f"  {g}")
    if not genres:
        print("  (none found)")


def _apply_rating(
    client: MediaServerClient | None,
    item_id: str,
    rating: str,
    label: str,
) -> str:
    """GET-then-POST round-trip to set OfficialRating. Returns action string."""
    if client is None:
        log.error(
            "_apply_rating called with no server client for %s (%s)", label, item_id
        )
        return "error"
    try:
        full_item = client.get_item(item_id)
        full_item["OfficialRating"] = rating
        client.update_item(item_id, full_item)
        verb = "Cleared rating from" if not rating else f"Set {rating} on"
        log.info("%s %s", verb, label)
        return "set"
    except MediaServerError as exc:
        log.error("Failed to update %s: %s", label, exc)
        return "error"


def _decide_rating_action(
    *,
    client: MediaServerClient | None,
    item_id: str | None,
    tier: str,
    current_rating: str,
    label: str,
    dry_run: bool,
    action_dry: str = "dry_run",
    action_already: str = "already_correct",
) -> str:
    """Common rating-decision logic for lyrics and genre passes."""
    if client is None:
        return "server_unavailable"
    if not item_id:
        if item_id == "":
            log.error("Server item for %s has empty 'Id'; cannot update", label)
        else:
            log.warning("Audio file not found in server: %s", label)
        return "not_found_in_server"
    if current_rating == tier:
        log.debug("Already rated %s: %s", tier, label)
        return action_already
    if dry_run:
        log.info("[DRY RUN] Would set %s on %s", tier, label)
        return action_dry
    return _apply_rating(client, item_id, tier, label)


def _decide_clear_action(
    *,
    client: MediaServerClient | None,
    item_id: str | None,
    current_rating: str,
    label: str,
    dry_run: bool,
) -> str:
    """Common clear-decision logic for lyrics pass."""
    if client is None:
        return "server_unavailable"
    if not item_id:
        if item_id == "":
            log.error("Server item for %s has empty 'Id'; cannot update", label)
        else:
            log.warning("Audio file not found in server: %s", label)
        return "not_found_in_server"
    if not current_rating:
        return "skipped"
    if dry_run:
        log.info("[DRY RUN] Would clear rating from %s", label)
        return "dry_run_clear"
    action = _apply_rating(client, item_id, "", label)
    return "cleared" if action == "set" else action


# ---------------------------------------------------------------------------
# CLI & Main
# ---------------------------------------------------------------------------


_RATE_EXAMPLES = """\
examples:
  # Dry run — analyze without touching the server
  %(prog)s --dry-run --report report.csv

  # Scope to a named library
  %(prog)s --library Music --dry-run

  # Skip tracks that already have a rating
  %(prog)s --library Music --skip-existing
"""

_FORCE_EXAMPLES = """\
examples:
  # Rate a known-clean library as G
  %(prog)s G --library Music

  # Dry-run force-rate using a named server
  %(prog)s G --server home-jellyfin --dry-run
"""

_RESET_EXAMPLES = """\
examples:
  # Remove all ratings from the Music library
  %(prog)s --library Music

  # Dry run — show what would be cleared
  %(prog)s --library Music --dry-run
"""

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


def build_parser() -> argparse.ArgumentParser:
    # --- shared parent for options common to all subcommands ---
    shared = argparse.ArgumentParser(add_help=False)
    shared.add_argument(
        "--config",
        default=None,
        help="Path to TOML config file (default: explicit_config.toml next to script)",
    )
    shared.add_argument(
        "--env-file",
        default=None,
        help="Path to .env file (default: .env in the repo root; e.g. --env-file .env.prod)",
    )
    shared.add_argument(
        "--server-url",
        default=None,
        help="Server URL for one-off use (requires --api-key)",
    )
    shared.add_argument(
        "--api-key",
        default=None,
        help="API key for one-off use (requires --server-url)",
    )
    shared.add_argument(
        "--server",
        action="append",
        default=None,
        dest="server",
        metavar="NAME",
        help="Target a named server (repeatable; e.g. --server home-emby --server home-jellyfin)",
    )
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
    shared.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Debug logging",
    )

    # --- top-level parser ---
    parser = argparse.ArgumentParser(
        prog="SetMusicParentalRating",
        description="Fetch lyrics from the Emby or Jellyfin API, detect explicit "
        "content, and set OfficialRating on matching audio tracks.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_MAIN_EXAMPLES,
    )
    parser.add_argument(
        "--version", action="version", version=f"%(prog)s {__version__}"
    )

    subparsers = parser.add_subparsers(dest="command")

    # --- rate subcommand ---
    rate_parser = subparsers.add_parser(
        "rate",
        parents=[shared],
        help="Fetch lyrics from server, detect explicit content, set ratings",
        description="Fetch lyrics from the media server API, detect explicit "
        "content, and set OfficialRating on matching tracks.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_RATE_EXAMPLES,
    )
    rate_parser.add_argument(
        "library_path",
        nargs="*",
        default=None,
        help="Library root directory/directories (overrides config; multiple paths supported)",
    )
    rate_parser.add_argument(
        "-n",
        "--dry-run",
        action="store_true",
        help="Analyze only — no server updates",
    )
    rate_parser.add_argument(
        "--report",
        default=None,
        help="CSV report output path",
    )
    rate_parser.add_argument(
        "--overwrite",
        action="store_true",
        default=None,
        help="Re-evaluate and update tracks that already have a rating",
    )
    rate_parser.add_argument(
        "--skip-existing",
        action="store_true",
        default=None,
        help="Skip tracks that already have any rating",
    )

    # --- force subcommand ---
    force_parser = subparsers.add_parser(
        "force",
        parents=[shared],
        help="Set a fixed rating on all tracks in scope (no lyrics evaluation)",
        description="Skip detection and set a fixed OfficialRating on ALL "
        "audio tracks in the configured scope.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_FORCE_EXAMPLES,
    )
    force_parser.add_argument(
        "rating",
        help="Rating to set on all tracks (e.g. G, PG-13, R)",
    )
    force_parser.add_argument(
        "library_path",
        nargs="*",
        default=None,
        help="Library root directory/directories (overrides config; multiple paths supported)",
    )
    force_parser.add_argument(
        "-n",
        "--dry-run",
        action="store_true",
        help="Analyze only — no server updates",
    )
    force_parser.add_argument(
        "--report",
        default=None,
        help="CSV report output path",
    )
    force_parser.add_argument(
        "--overwrite",
        action="store_true",
        default=None,
        help="Overwrite existing ratings",
    )
    force_parser.add_argument(
        "--skip-existing",
        action="store_true",
        default=None,
        help="Skip tracks that already have any rating",
    )

    # --- reset subcommand ---
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

    return parser


def setup_logging(verbose: bool) -> None:
    level = logging.DEBUG if verbose else logging.INFO
    logging.basicConfig(
        level=level,
        format="%(asctime)s %(levelname)-8s %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )


def print_summary(results: list[DetectionResult], label: str = "") -> None:
    if label:
        print(f"\n=== {label} ===")
    scan_results = [
        r
        for r in results
        if r.source in ("sidecar", "embedded", "lyrics") or r.action == "no_audio_file"
    ]
    genre_results = [
        r
        for r in results
        if r.action in ("g_genre", "g_genre_already_correct", "dry_run_g_genre")
    ]
    sidecar_count = sum(
        1
        for r in scan_results
        if r.sidecar_path is not None
        or r.source == "lyrics"
        or r.action == "no_audio_file"
    )
    total = len(scan_results)
    r_count = sum(1 for r in scan_results if r.tier == "R")
    pg13_count = sum(1 for r in scan_results if r.tier == "PG-13")
    clean = sum(1 for r in scan_results if r.tier is None)
    audio_found = sum(
        1
        for r in scan_results
        if r.sidecar_path is not None
        and r.audio_path is not None
        and r.action != "no_audio_file"
    )
    sidecar_server_matched = sum(
        1 for r in scan_results if r.sidecar_path is not None and r.server_item_id
    )
    rated = sum(1 for r in results if r.action == "set")
    already = sum(1 for r in results if r.action == "already_correct")
    cleared = sum(1 for r in results if r.action == "cleared")
    dry = sum(1 for r in results if r.action.startswith("dry_run"))
    errors = sum(1 for r in results if r.action == "error")
    server_unavail = sum(1 for r in results if r.action == "server_unavailable")
    g_genre_rated = sum(1 for r in genre_results if r.action == "g_genre")
    g_genre_already = sum(
        1 for r in genre_results if r.action == "g_genre_already_correct"
    )
    g_genre_dry = sum(1 for r in genre_results if r.action == "dry_run_g_genre")

    print()
    print("=== Explicit Lyrics Scan Complete ===")
    if total:
        has_sidecar_results = any(r.sidecar_path is not None for r in scan_results)
        if sidecar_count:
            lyrics_label = (
                "Sidecars scanned" if has_sidecar_results else "Lyrics evaluated"
            )
            print(f"  {lyrics_label}:    {sidecar_count}")
        print(f"    R-rated:           {r_count}")
        print(f"    PG-13:             {pg13_count}")
        print(f"    Clean:             {clean}")
        if sidecar_count and has_sidecar_results:
            print(f"  Audio files found:   {audio_found} / {sidecar_count}")
            print(f"  Server items matched: {sidecar_server_matched} / {audio_found}")
    print(f"  Ratings set:         {rated}")
    print(f"  Already correct:     {already}")
    print(f"  Ratings cleared:     {cleared}")
    if g_genre_rated or g_genre_already or g_genre_dry:
        print(f"  G (genre-matched):   {g_genre_rated}")
        print(f"  Already G (genre):   {g_genre_already}")
        if g_genre_dry:
            print(f"  Dry-run G (genre):   {g_genre_dry}")
    if server_unavail:
        print(f"  Server unavailable:  {server_unavail}")
    if dry:
        print(f"  Dry-run would act:   {dry}")
    print(f"  Errors:              {errors}")


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

    all_results: list[DetectionResult] = []
    multi = len(config.servers) > 1
    had_failure = False

    for server in config.servers:
        srv_config = replace(
            config,
            server_url=server.url,
            server_api_key=server.api_key,
            server_type=server.server_type,
        )
        label = f"{server.name} ({server.server_type.title()})" if multi else ""
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
                log.error("Unknown command: %s", args.command)
                sys.exit(1)
        except SystemExit as exc:
            log.error("%s failed (exit %s).", label or "Server", exc.code)
            results = []
            had_failure = True

        all_results.extend(results)
        if multi:
            print_summary(results, label=label)

    if config.report_path:
        write_report(all_results, config.report_path, config.library_paths)

    if not multi:
        print_summary(all_results)

    if had_failure:
        sys.exit(1)


if __name__ == "__main__":
    main()
