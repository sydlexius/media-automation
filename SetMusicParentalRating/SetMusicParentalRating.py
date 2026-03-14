#!/usr/bin/env python3
"""SetMusicParentalRating — scan sidecar lyric files for explicit content and
set OfficialRating on matching audio tracks via the Emby or Jellyfin API.

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

AUDIO_EXTENSIONS = (
    ".flac",
    ".mp3",
    ".m4a",
    ".ogg",
    ".opus",
    ".wma",
    ".wav",
    ".aac",
    ".alac",
    ".wv",
    ".ape",
)
SIDECAR_EXTENSIONS = frozenset({".lrc", ".txt"})

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
class Config:
    library_paths: list[Path]
    server_url: str  # "" in "both" mode; use emby_url/jellyfin_url instead
    server_api_key: str  # "" in "both" mode; use emby_api_key/jellyfin_api_key instead
    server_type: str = "emby"
    r_stems: list[str] = field(default_factory=lambda: list(DEFAULT_R_STEMS))
    r_exact: list[str] = field(default_factory=lambda: list(DEFAULT_R_EXACT))
    pg13_stems: list[str] = field(default_factory=lambda: list(DEFAULT_PG13_STEMS))
    pg13_exact: list[str] = field(default_factory=lambda: list(DEFAULT_PG13_EXACT))
    false_positives: list[str] = field(
        default_factory=lambda: list(DEFAULT_FALSE_POSITIVES)
    )
    dry_run: bool = False
    clear: bool = False
    force_rating: str | None = None
    report_path: Path | None = None
    g_genres: list[str] = field(default_factory=list)
    embedded_lyrics: bool = False
    lyrics_priority: str = "sidecar"  # "sidecar" | "embedded" | "most_explicit"
    # Per-server credentials — populated from env vars, .env file, or TOML
    # [emby]/[jellyfin] sections; used by main() to build derived configs in
    # --server-type both mode.
    emby_url: str = ""
    emby_api_key: str = ""
    jellyfin_url: str = ""
    jellyfin_api_key: str = ""

    def __post_init__(self) -> None:
        if self.server_type not in ("emby", "jellyfin", "both"):
            raise ValueError(
                f"server_type must be 'emby', 'jellyfin', or 'both', got {self.server_type!r}"
            )
        if self.lyrics_priority not in ("sidecar", "embedded", "most_explicit"):
            raise ValueError(
                f"lyrics_priority must be 'sidecar', 'embedded', or 'most_explicit', "
                f"got {self.lyrics_priority!r}"
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
    source: str = ""  # "sidecar" | "embedded" | "genre" | "force"
    source_conflict: str = (
        ""  # format: "{loser}:{tier}->{WINNER}:{tier}"; loser lowercase, WINNER uppercase;
        #  tier is "R"|"PG-13"|"clean" (clean = no explicit content); empty when sources agree
    )
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
        has_emby = bool((os.environ.get("EMBY_URL") or "").strip()) and bool(
            (os.environ.get("EMBY_API_KEY") or "").strip()
        )
        has_jellyfin = bool((os.environ.get("JELLYFIN_URL") or "").strip()) and bool(
            (os.environ.get("JELLYFIN_API_KEY") or "").strip()
        )
        if has_emby or has_jellyfin:
            log.debug(
                ".env file not found at %s (credentials provided via environment variables)",
                path,
            )
        else:
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

    if not raw_paths and getattr(args, "command", None) != "genres":
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

    # --- server_type (explicit override, or auto-detected from which env vars are set) ---
    explicit_type = (
        (
            (getattr(args, "server_type", None) or "")
            or os.environ.get("SERVER_TYPE", "")
            or env_file.get("SERVER_TYPE", "")
            or str(toml.get("general", {}).get("server_type", "") or "")
        )
        .lower()
        .strip()
    )

    # --- Resolve both credential pairs unconditionally ---
    cli_server_url = getattr(args, "server_url", None) or ""
    cli_api_key = getattr(args, "api_key", None) or ""

    emby_url = (
        os.environ.get("EMBY_URL", "")
        or env_file.get("EMBY_URL", "")
        or toml.get("emby", {}).get("url", "")
    ).strip()
    emby_api_key = (
        os.environ.get("EMBY_API_KEY", "") or env_file.get("EMBY_API_KEY", "") or ""
    ).strip()
    jellyfin_url = (
        os.environ.get("JELLYFIN_URL", "")
        or env_file.get("JELLYFIN_URL", "")
        or toml.get("jellyfin", {}).get("url", "")
    ).strip()
    jellyfin_api_key = (
        os.environ.get("JELLYFIN_API_KEY", "")
        or env_file.get("JELLYFIN_API_KEY", "")
        or ""
    ).strip()

    has_emby_url = bool(emby_url)
    has_jellyfin_url = bool(jellyfin_url)

    if explicit_type:
        server_type = explicit_type
        if server_type == "both":
            if cli_server_url or cli_api_key:
                print(
                    "Error: --server-url and --api-key are not supported with --server-type both. "
                    "Set credentials via EMBY_URL/EMBY_API_KEY and JELLYFIN_URL/JELLYFIN_API_KEY.",
                    file=sys.stderr,
                )
                sys.exit(1)
            if not emby_url or not emby_api_key:
                print(
                    "Error: --server-type both requires EMBY_URL and EMBY_API_KEY",
                    file=sys.stderr,
                )
                sys.exit(1)
            if not jellyfin_url or not jellyfin_api_key:
                print(
                    "Error: --server-type both requires JELLYFIN_URL and JELLYFIN_API_KEY",
                    file=sys.stderr,
                )
                sys.exit(1)
    else:
        if has_emby_url and has_jellyfin_url:
            print(
                "Error: both Emby and Jellyfin are configured; "
                "use --server-type emby, --server-type jellyfin, or --server-type both to select.",
                file=sys.stderr,
            )
            sys.exit(1)
        server_type = "jellyfin" if has_jellyfin_url else "emby"

    # --- server_url / server_api_key for single-server mode ---
    if server_type == "both":
        # Unused in "both" mode — main() uses per-server fields from Config directly
        server_url = ""
        server_api_key = ""
    elif server_type == "jellyfin":
        server_url = cli_server_url or jellyfin_url
        server_api_key = cli_api_key or jellyfin_api_key
    else:
        server_url = cli_server_url or emby_url
        server_api_key = cli_api_key or emby_api_key

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
    embedded_lyrics = det.get("embedded_lyrics", False)
    lyrics_priority_toml = det.get("lyrics_priority", "sidecar")

    # --- report ---
    report_path_str = getattr(args, "report", None) or toml.get("report", {}).get(
        "output_path"
    )
    report_path = Path(report_path_str) if report_path_str else None

    return Config(
        library_paths=library_paths,
        server_url=server_url.rstrip("/"),
        server_api_key=server_api_key,
        server_type=server_type,
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
        embedded_lyrics=embedded_lyrics
        if getattr(args, "embedded_lyrics", None) is None
        else args.embedded_lyrics,
        lyrics_priority=lyrics_priority_toml
        if getattr(args, "lyrics_priority", None) is None
        else args.lyrics_priority,
        emby_url=emby_url.rstrip("/"),
        emby_api_key=emby_api_key,
        jellyfin_url=jellyfin_url.rstrip("/"),
        jellyfin_api_key=jellyfin_api_key,
    )


# ---------------------------------------------------------------------------
# Filesystem Scanning
# ---------------------------------------------------------------------------


def find_sidecars(library_path: Path) -> list[Path]:
    """Find all .lrc and .txt sidecar files under the library path."""
    sidecars: list[Path] = []
    for ext in SIDECAR_EXTENSIONS:
        sidecars.extend(library_path.rglob(f"*{ext}"))
    sidecars.sort()
    return sidecars


def match_audio_file(sidecar: Path) -> Path | None:
    """Find the audio file in the same directory with the same stem."""
    for ext in AUDIO_EXTENSIONS:
        candidate = sidecar.with_suffix(ext)
        if candidate.is_file():
            return candidate
    return None


def scan_library(library_path: Path) -> list[tuple[Path, Path | None]]:
    """Return (sidecar, audio_file_or_None) pairs."""
    sidecars = find_sidecars(library_path)
    log.info("Found %d sidecar files under %s", len(sidecars), library_path)
    results: list[tuple[Path, Path | None]] = []
    for sc in sidecars:
        audio = match_audio_file(sc)
        if audio is None:
            log.warning("No matching audio file for sidecar: %s", sc)
        else:
            log.debug("Matched: %s -> %s", sc.name, audio.name)
        results.append((sc, audio))
    return results


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


def parse_sidecar(path: Path) -> str:
    """Read a sidecar file and return clean text content."""
    try:
        raw = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        log.warning("Could not read sidecar %s: %s", path, exc)
        return ""
    return strip_lrc_tags(raw)


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


_TIER_RANK: dict[str | None, int] = {None: 0, "PG-13": 1, "R": 2}


def _resolve_priority(
    sidecar_tier: str | None,
    sidecar_matched: list[str],
    embedded_tier: str | None,
    embedded_matched: list[str],
    priority: str,
) -> tuple[str | None, list[str], str, str]:
    """Return (winning_tier, winning_matched, winning_source, source_conflict).

    winning_source is lowercase ("sidecar" or "embedded") for storage in
    DetectionResult.source. source_conflict uses uppercase for the winning
    source name and lowercase for the losing source name (e.g.
    "embedded:PG-13->SIDECAR:R"); empty string when both sources agree.
    None tier is serialized as "clean" in source_conflict.
    Equal tiers always defer to sidecar regardless of priority mode.
    """

    def _label(tier: str | None) -> str:
        return tier if tier is not None else "clean"

    if sidecar_tier == embedded_tier:
        # Both agree — no conflict, sidecar wins by convention
        return sidecar_tier, sidecar_matched, "sidecar", ""

    # Sources disagree — compute conflict string regardless of policy
    if priority == "sidecar":
        winner, w_matched, w_src = sidecar_tier, sidecar_matched, "sidecar"
        loser_src, loser_tier = "embedded", embedded_tier
    elif priority == "embedded":
        winner, w_matched, w_src = embedded_tier, embedded_matched, "embedded"
        loser_src, loser_tier = "sidecar", sidecar_tier
    else:  # most_explicit
        if _TIER_RANK[embedded_tier] > _TIER_RANK[sidecar_tier]:
            winner, w_matched, w_src = embedded_tier, embedded_matched, "embedded"
            loser_src, loser_tier = "sidecar", sidecar_tier
        else:
            winner, w_matched, w_src = sidecar_tier, sidecar_matched, "sidecar"
            loser_src, loser_tier = "embedded", embedded_tier

    conflict = f"{loser_src}:{_label(loser_tier)}->{w_src.upper()}:{_label(winner)}"
    return winner, w_matched, w_src, conflict


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

    def prefetch_audio_items(
        self, *, include_media_sources: bool = False
    ) -> dict[str, dict]:
        """Paginated fetch of all Audio items. Returns {normalized_path: item}.

        Pass include_media_sources=True to append MediaSources to the Fields
        parameter on Emby (includes embedded lyrics in MediaStreams.Extradata at
        the cost of a larger payload). On Jellyfin this flag has no effect —
        embedded lyrics are fetched per-track via /Audio/{id}/Lyrics by the caller.
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
            result = self._request(
                "GET",
                f"/Users/{uid}/Items?Recursive=true&IncludeItemTypes=Audio"
                f"&Fields={fields}"
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
        has no lyrics or the request fails. Requires no lyrics plugin.
        """
        try:
            data = self._request("GET", f"/Audio/{item_id}/Lyrics")
        except MediaServerError as exc:
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
                    "source_conflict",
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
                        r.source_conflict,
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


def process_library(config: Config) -> list[DetectionResult]:
    """Main flow: scan sidecars -> detect -> update media server.

    Thread-safety: this function is single-threaded by design. The shared
    ``handled_paths`` set and ``results`` list are not thread-safe. If
    parallelism is added (e.g. concurrent Jellyfin lyrics fetches), these
    would need locking or per-thread accumulation.
    """
    _validate_library_paths(config.library_paths)

    pairs: list[tuple[Path, Path | None]] = []
    seen: set[Path] = set()
    for lp in config.library_paths:
        for pair in scan_library(lp):
            if pair[0] not in seen:
                seen.add(pair[0])
                pairs.append(pair)

    # Prefetch server items for path matching (even in dry-run, we read but don't write)
    client: MediaServerClient | None = None
    server_items: dict[str, dict] = {}
    if config.server_url and config.server_api_key:
        client = MediaServerClient(
            config.server_url, config.server_api_key, config.server_type
        )
        try:
            server_items = client.prefetch_audio_items(
                include_media_sources=config.embedded_lyrics
            )
        except MediaServerError as exc:
            log.error("Failed to prefetch server items: %s", exc)
            log.error("Continuing in analysis-only mode")
            client = None
    else:
        log.info("Server URL or API key not configured; running in analysis-only mode")

    results: list[DetectionResult] = []
    handled_paths: set[str] = set()

    for sidecar, audio in pairs:
        text = parse_sidecar(sidecar)
        tier, matched = classify_lyrics(text, config)

        dr = DetectionResult(
            sidecar_path=sidecar,
            audio_path=audio,
            tier=tier,
            matched_words=matched,
            source="sidecar",
        )

        if tier:
            log.info("%s -> %s (words: %s)", sidecar.name, tier, ", ".join(matched))
        else:
            log.debug("%s -> clean", sidecar.name)

        if audio is None:
            dr.action = "no_audio_file"
            results.append(dr)
            continue

        # Resolve server item
        norm_audio = _normalize_path(str(audio))
        handled_paths.add(norm_audio)
        server_item = server_items.get(norm_audio)
        if server_item:
            item_id, prev_rating, artist, album = _item_fields(server_item)
            dr.server_item_id = item_id
            dr.previous_rating = prev_rating
            dr.artist = artist
            dr.album = album

        # Augment sidecar classification with embedded lyrics if present
        if config.embedded_lyrics and server_item and client is not None:
            try:
                if config.server_type == "jellyfin":
                    _sid = server_item.get("Id", "")
                    embedded_text = client.fetch_lyrics_jellyfin(_sid) if _sid else ""
                else:
                    embedded_text = extract_embedded_lyrics(server_item)
                if embedded_text:
                    emb_tier, emb_matched = classify_lyrics(embedded_text, config)
                    tier, matched, winning_source, dr.source_conflict = (
                        _resolve_priority(
                            tier, matched, emb_tier, emb_matched, config.lyrics_priority
                        )
                    )
                    dr.source = winning_source
                    dr.tier = tier
                    dr.matched_words = matched
            except Exception as exc:
                log.warning(
                    "Failed to augment sidecar result with embedded lyrics for %s: %s",
                    audio,
                    exc,
                )

        # Decide action
        if tier is not None:
            dr.action = _decide_rating_action(
                client=client,
                item_id=dr.server_item_id,
                tier=tier,
                current_rating=dr.previous_rating,
                label=str(audio),
                dry_run=config.dry_run,
            )
        elif config.clear:
            dr.action = _decide_clear_action(
                client=client,
                item_id=dr.server_item_id,
                current_rating=dr.previous_rating,
                label=str(audio),
                dry_run=config.dry_run,
            )
        else:
            dr.action = "skipped"

        results.append(dr)

    # Compute once — shared by embedded and genre passes
    lib_roots = [Path(_normalize_path(str(lp))) for lp in config.library_paths]

    # --- Embedded lyrics pass ---
    if config.embedded_lyrics and client is not None:
        if config.server_type == "jellyfin":
            candidate_count = sum(
                1
                for p in server_items
                if p not in handled_paths and _is_under_roots(p, lib_roots)
            )
            if candidate_count:
                log.info(
                    "Embedded lyrics pass: querying Jellyfin for %d tracks individually"
                    " (one request per track)...",
                    candidate_count,
                )
        for norm_path, item in server_items.items():
            if norm_path in handled_paths:
                continue
            if not _is_under_roots(norm_path, lib_roots):
                continue
            if config.server_type == "jellyfin":
                _iid = item.get("Id", "")
                text = client.fetch_lyrics_jellyfin(_iid) if _iid else ""
            else:
                text = extract_embedded_lyrics(item)
            if not text:
                continue
            tier, matched = classify_lyrics(text, config)
            item_id, prev_rating, artist, album = _item_fields(item)
            dr = DetectionResult(
                sidecar_path=None,
                audio_path=Path(norm_path),
                tier=tier,
                matched_words=matched,
                server_item_id=item_id,
                previous_rating=prev_rating,
                artist=artist,
                album=album,
                source="embedded",
            )
            if item_id is None:
                log.warning(
                    "Embedded-lyrics pass: server item at %s has no 'Id' field;"
                    " cannot update",
                    norm_path,
                )
                dr.action = "not_found_in_server"
            elif item_id == "":
                log.error(
                    "Embedded-lyrics pass: server item at %s has empty 'Id';"
                    " cannot update",
                    norm_path,
                )
                dr.action = "not_found_in_server"
            elif tier is not None:
                dr.action = _decide_rating_action(
                    client=client,
                    item_id=item_id,
                    tier=tier,
                    current_rating=prev_rating,
                    label=norm_path,
                    dry_run=config.dry_run,
                )
            elif config.clear:
                dr.action = _decide_clear_action(
                    client=client,
                    item_id=item_id,
                    current_rating=prev_rating,
                    label=norm_path,
                    dry_run=config.dry_run,
                )
            else:
                dr.action = "skipped"
            handled_paths.add(norm_path)
            results.append(dr)

    # --- Genre-based G rating pass ---
    if config.g_genres and client is not None:
        for norm_path, item in server_items.items():
            if norm_path in handled_paths:
                continue
            if not _is_under_roots(norm_path, lib_roots):
                continue
            matched_genre = match_g_genre(item, config.g_genres)
            if matched_genre is None:
                continue
            item_id, prev_rating, artist, album = _item_fields(item)
            if item_id is None:
                log.warning(
                    "Genre-pass: server item at %s has no 'Id' field; skipping",
                    norm_path,
                )
                continue
            if item_id == "":
                log.error(
                    "Genre-pass: server item at %s has empty 'Id'; skipping",
                    norm_path,
                )
                continue
            dr = DetectionResult(
                sidecar_path=None,
                audio_path=Path(norm_path),
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
    """'rate' subcommand: set a fixed rating on ALL audio tracks under the
    library path(s), skipping tracks already at the target rating."""
    _validate_library_paths(config.library_paths)
    if not config.server_url or not config.server_api_key:
        log.error(
            "'rate' requires a server URL and API key "
            "(set EMBY_URL+EMBY_API_KEY or JELLYFIN_URL+JELLYFIN_API_KEY in .env, or use --server-url/--api-key)"
        )
        sys.exit(1)

    target = config.force_rating
    client = MediaServerClient(
        config.server_url, config.server_api_key, config.server_type
    )
    try:
        all_items = client.prefetch_audio_items()
    except MediaServerError as exc:
        log.error("Failed to prefetch server items: %s", exc)
        sys.exit(1)

    # Filter to items under the library path(s) (path-aware, avoids /music matching /music2)
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


def list_genres_mode(config: Config) -> None:
    """'genres' subcommand: print all Audio genre names from the server to stdout. Exits with non-zero status on error."""
    if config.server_type == "both":
        print(
            "Error: 'genres' is not supported with --server-type both. "
            "Run separately with --server-type emby and --server-type jellyfin.",
            file=sys.stderr,
        )
        sys.exit(1)
    if not config.server_url or not config.server_api_key:
        print(
            "Error: 'genres' requires server URL and API key "
            "(set EMBY_URL+EMBY_API_KEY or JELLYFIN_URL+JELLYFIN_API_KEY in .env, or use --server-url/--api-key). "
            "Use --server-type to select Emby or Jellyfin when both are configured.",
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
    """Common rating-decision logic for sidecar, embedded, and genre passes."""
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
    """Common clear-decision logic for sidecar and embedded passes."""
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


_SCAN_EXAMPLES = """\
examples:
  # Dry run — analyze without touching the server
  %(prog)s /path/to/music --dry-run --report report.csv

  # Multiple library paths in a single run
  %(prog)s /path/to/music /path/to/classical --dry-run

  # Include embedded lyrics scanning
  %(prog)s /path/to/music --embedded-lyrics --lyrics-priority most_explicit

  # Clear stale ratings after fixing sidecar typos
  %(prog)s /path/to/music --clear
"""

_RATE_EXAMPLES = """\
examples:
  # Rate a known-clean library as G
  %(prog)s /path/to/classical G

  # Dry-run rate against Jellyfin
  %(prog)s /path/to/classical G --server-type jellyfin --dry-run
"""

_GENRES_EXAMPLES = """\
examples:
  # List all genre tags from the default (Emby) server
  %(prog)s

  # List genre tags from a Jellyfin server
  %(prog)s --server-type jellyfin
"""

_MAIN_EXAMPLES = """\
subcommands:
  scan    Scan sidecar/embedded lyrics and set ratings
  rate    Set a fixed rating on all tracks under the given path(s)
  genres  List all Audio genre tags from the server

examples:
  %(prog)s scan /path/to/music --dry-run --report report.csv
  %(prog)s rate /path/to/classical G
  %(prog)s genres --server-type jellyfin
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
        "--server-type",
        default=None,
        choices=("emby", "jellyfin", "both"),
        help="Media server type: 'emby', 'jellyfin', or 'both' (syncs both in one pass)",
    )
    shared.add_argument(
        "--server-url",
        default=None,
        help="Server URL — overrides the env var for the active server type",
    )
    shared.add_argument(
        "--api-key",
        default=None,
        help="API key — overrides the env var for the active server type",
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
        description="Scan sidecar lyric files for explicit content and set "
        "OfficialRating on matching tracks via the Emby or Jellyfin API.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_MAIN_EXAMPLES,
    )
    parser.add_argument(
        "--version", action="version", version=f"%(prog)s {__version__}"
    )

    subparsers = parser.add_subparsers(dest="command")

    # --- scan subcommand ---
    scan_parser = subparsers.add_parser(
        "scan",
        parents=[shared],
        help="Scan sidecar/embedded lyrics and set ratings",
        description="Scan sidecar and embedded lyrics for explicit content, "
        "then set OfficialRating on matching tracks.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_SCAN_EXAMPLES,
    )
    scan_parser.add_argument(
        "library_path",
        nargs="*",
        default=None,
        help="Library root directory/directories (overrides config; multiple paths supported)",
    )
    scan_parser.add_argument(
        "-n",
        "--dry-run",
        action="store_true",
        help="Analyze only — no server updates",
    )
    scan_parser.add_argument(
        "--report",
        default=None,
        help="CSV report output path",
    )
    scan_parser.add_argument(
        "--clear",
        action="store_true",
        help="Clear ratings from tracks whose sidecars exist but contain no explicit words",
    )
    scan_parser.add_argument(
        "--embedded-lyrics",
        action=argparse.BooleanOptionalAction,
        default=None,
        help=(
            "Scan embedded tag lyrics for explicit content. "
            "On Emby, adds MediaSources to the bulk prefetch. "
            "On Jellyfin, adds one GET /Audio/{id}/Lyrics request per track in scope — "
            "including sidecar-matched tracks (for --lyrics-priority resolution). "
            "(default: off)"
        ),
    )
    scan_parser.add_argument(
        "--lyrics-priority",
        default=None,
        choices=("sidecar", "embedded", "most_explicit"),
        help=(
            "Which source wins when a track has both a sidecar (.lrc/.txt) and embedded lyrics. "
            "Only applies when --embedded-lyrics is on. "
            "Default: sidecar. most_explicit picks whichever detected the higher tier. "
            "Ties (both sources at the same tier) always defer to sidecar regardless of priority."
        ),
    )

    # --- rate subcommand ---
    rate_parser = subparsers.add_parser(
        "rate",
        parents=[shared],
        help="Set a fixed rating on all tracks under the given path(s)",
        description="Skip detection and set a fixed OfficialRating on ALL "
        "audio tracks under the given library path(s).",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_RATE_EXAMPLES,
    )
    rate_parser.add_argument(
        "library_path",
        nargs="+",
        help="Library root directory/directories",
    )
    rate_parser.add_argument(
        "rating",
        help="Rating to set on all tracks (e.g. G, PG-13, R)",
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

    # --- genres subcommand ---
    subparsers.add_parser(
        "genres",
        parents=[shared],
        help="List all Audio genre tags from the server",
        description="Connect to the media server, print all Audio genre tags, "
        "then exit. Useful for populating [detection.g_genres] in the config.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=_GENRES_EXAMPLES,
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
        if r.source in ("sidecar", "embedded") or r.action == "no_audio_file"
    ]
    genre_results = [
        r
        for r in results
        if r.action in ("g_genre", "g_genre_already_correct", "dry_run_g_genre")
    ]
    sidecar_count = sum(
        1
        for r in scan_results
        if r.sidecar_path is not None or r.action == "no_audio_file"
    )
    embedded_only_count = sum(
        1 for r in scan_results if r.sidecar_path is None and r.source == "embedded"
    )
    sidecar_won_count = sum(
        1
        for r in scan_results
        if r.sidecar_path is not None
        and r.source_conflict != ""
        and r.source == "sidecar"
    )
    embedded_won_count = sum(
        1
        for r in scan_results
        if r.sidecar_path is not None
        and r.source_conflict != ""
        and r.source == "embedded"
    )
    conflict_count = sum(1 for r in scan_results if r.source_conflict != "")
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
    embedded_server_matched = sum(
        1
        for r in scan_results
        if r.sidecar_path is None and r.source == "embedded" and r.server_item_id
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
        if sidecar_count:
            print(f"  Sidecars scanned:    {sidecar_count}")
        if sidecar_count and conflict_count:
            print(f"    Used sidecar lyrics:  {sidecar_won_count}")
            print(f"    Used embedded lyrics: {embedded_won_count}")
        if embedded_only_count:
            print(f"  From embedded tags only: {embedded_only_count}")
        print(f"    R-rated:           {r_count}")
        print(f"    PG-13:             {pg13_count}")
        print(f"    Clean:             {clean}")
        if sidecar_count:
            print(f"  Audio files found:   {audio_found} / {sidecar_count}")
            print(f"  Server items matched: {sidecar_server_matched} / {audio_found}")
        if embedded_only_count:
            print(
                f"  Embedded matched:     {embedded_server_matched} / {embedded_only_count}"
            )
        if conflict_count:
            print(
                f"  Source conflicts:    {conflict_count}"
                f"  ← check source_conflict column in report"
            )
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

    # ValueError is raised by Config.__post_init__ for invalid field values
    # (e.g. unrecognised server_type). Other config errors call sys.exit() directly.
    try:
        config = build_config(args)
    except ValueError as exc:
        parser.error(str(exc))

    if args.command == "genres":
        list_genres_mode(config)
        return

    if config.server_type == "both":
        emby_cfg = replace(
            config,
            server_type="emby",
            server_url=config.emby_url,
            server_api_key=config.emby_api_key,
        )
        jf_cfg = replace(
            config,
            server_type="jellyfin",
            server_url=config.jellyfin_url,
            server_api_key=config.jellyfin_api_key,
        )
        log.info("--- Starting Emby run ---")
        if config.force_rating:
            try:
                emby_results = force_rate_library(emby_cfg)
            except SystemExit as exc:
                log.error(
                    "Emby run failed (exit code %s); Jellyfin run will still proceed.",
                    exc.code,
                )
                emby_results = []
        else:
            try:
                emby_results = process_library(emby_cfg)
            except SystemExit as exc:
                log.error(
                    "Emby run failed (exit code %s); Jellyfin run will still proceed.",
                    exc.code,
                )
                emby_results = []

        log.info("--- Starting Jellyfin run ---")
        if config.force_rating:
            try:
                jf_results = force_rate_library(jf_cfg)
            except SystemExit as exc:
                log.error("Jellyfin run failed (exit code %s).", exc.code)
                jf_results = []
        else:
            try:
                jf_results = process_library(jf_cfg)
            except SystemExit as exc:
                log.error("Jellyfin run failed (exit code %s).", exc.code)
                jf_results = []
        results = emby_results + jf_results
        if config.report_path:
            write_report(results, config.report_path, config.library_paths)
        print_summary(emby_results, label="Emby")
        print_summary(jf_results, label="Jellyfin")
    else:
        if config.force_rating:
            results = force_rate_library(config)
        else:
            results = process_library(config)
        if config.report_path:
            write_report(results, config.report_path, config.library_paths)
        print_summary(results)


if __name__ == "__main__":
    main()
