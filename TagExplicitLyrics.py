#!/usr/bin/env python3
"""Scan sidecar lyric files for explicit content and set OfficialRating on
matching tracks via the Emby API.

Python 3.11+ recommended (uses tomllib from stdlib).
On older Python, falls back to the tomli package.
"""

from __future__ import annotations

import argparse
import csv
import json
import logging
import os
import re
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass, field
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
    "nigga",
    "nigger",
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


class EmbyAPIError(Exception):
    """Raised when an Emby API call fails."""


# ---------------------------------------------------------------------------
# Dataclasses
# ---------------------------------------------------------------------------


@dataclass
class Config:
    library_path: Path
    emby_url: str
    emby_api_key: str
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

    def __post_init__(self) -> None:
        self._r_exact_patterns = _compile_exact_patterns(self.r_exact)
        self._pg13_exact_patterns = _compile_exact_patterns(self.pg13_exact)
        self.g_genres = [g for g in self.g_genres if g.strip()]


@dataclass
class DetectionResult:
    sidecar_path: Path | None
    audio_path: Path | None
    tier: str | None  # "R", "PG-13", or None (clean)
    matched_words: list[str] = field(default_factory=list)
    emby_item_id: str | None = None
    action: str = ""  # set | cleared | skipped | already_correct | not_found_in_emby |
    #                    emby_unavailable | error | no_audio_file | dry_run | dry_run_clear |
    #                    g_genre | g_genre_already_correct | dry_run_g_genre
    previous_rating: str = ""
    artist: str = ""
    album: str = ""


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------


def load_env(path: Path, *, required: bool = False) -> dict[str, str]:
    """Parse a .env file into a dict. Skips comments and blank lines."""
    env: dict[str, str] = {}
    if not path.is_file():
        log.debug(".env file not found at %s", path)
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
        env_path = script_dir / ".env"
    env_file = load_env(env_path, required=bool(args.env_file))

    # --- library_path ---
    library_path_str = (
        args.library_path
        or os.environ.get("TAGLRC_LIBRARY_PATH")
        or env_file.get("TAGLRC_LIBRARY_PATH")
        or toml.get("general", {}).get("library_path")
    )
    if not library_path_str and not getattr(args, "list_genres", False):
        print(
            "Error: library_path is required. Provide it via command-line argument, "
            "TAGLRC_LIBRARY_PATH environment variable, or [general].library_path in the TOML config.",
            file=sys.stderr,
        )
        sys.exit(1)
    library_path = Path(library_path_str or ".")

    # --- emby_url ---
    emby_url = (
        args.emby_url
        or os.environ.get("EMBY_URL")
        or env_file.get("EMBY_URL")
        or toml.get("emby", {}).get("url", "")
    )

    # --- emby_api_key ---
    emby_api_key = (
        args.emby_api_key
        or os.environ.get("EMBY_API_KEY")
        or env_file.get("EMBY_API_KEY")
        or ""
    )

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
    report_path_str = args.report or toml.get("report", {}).get("output_path")
    report_path = Path(report_path_str) if report_path_str else None

    return Config(
        library_path=library_path,
        emby_url=emby_url.rstrip("/"),
        emby_api_key=emby_api_key,
        r_stems=r_stems,
        r_exact=r_exact,
        pg13_stems=pg13_stems,
        pg13_exact=pg13_exact,
        false_positives=false_positives,
        dry_run=args.dry_run,
        clear=args.clear,
        force_rating=args.force_rating,
        report_path=report_path,
        g_genres=g_genres,
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


class EmbyClient:
    """Minimal Emby HTTP client using urllib (stdlib)."""

    def __init__(self, base_url: str, api_key: str) -> None:
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
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
        req.add_header("X-Emby-Token", self.api_key)
        req.add_header("Content-Type", "application/json")
        req.add_header("Accept", "application/json")
        log.debug("Emby %s %s", method, url)
        try:
            with urllib.request.urlopen(req, timeout=15) as resp:
                resp_data = resp.read()
                if resp_data:
                    try:
                        return json.loads(resp_data)
                    except json.JSONDecodeError as exc:
                        raise EmbyAPIError(
                            f"Non-JSON response on {method} {path}: {resp_data[:200]!r}"
                        ) from exc
                return None
        except urllib.error.HTTPError as exc:
            body_snippet = ""
            try:
                body_snippet = exc.read().decode("utf-8", errors="replace")[:1024]
            except Exception as inner_exc:
                log.debug("Could not read HTTP error body: %s", inner_exc)
            raise EmbyAPIError(
                f"HTTP {exc.code} on {method} {path}: {body_snippet}"
            ) from exc
        except urllib.error.URLError as exc:
            raise EmbyAPIError(
                f"Connection error on {method} {path}: {exc.reason}"
            ) from exc

    def prefetch_audio_items(self) -> dict[str, dict]:
        """Paginated fetch of all Audio items. Returns {normalized_path: item}."""
        items_by_path: dict[str, dict] = {}
        start_index = 0
        page_size = 500
        while True:
            result = self._request(
                "GET",
                f"/Items?Recursive=true&IncludeItemTypes=Audio"
                f"&Fields=Path,OfficialRating,AlbumArtist,Album,Genres"
                f"&StartIndex={start_index}&Limit={page_size}",
            )
            if not result:
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
        log.info("Prefetched %d audio items from Emby", len(items_by_path))
        return items_by_path

    def _get_user_id(self) -> str:
        """Fetch and cache the first user ID (needed for user-scoped endpoints)."""
        if self._user_id is None:
            users = self._request("GET", "/Users")
            if not users:
                raise EmbyAPIError("No users returned from /Users")
            user_id = users[0].get("Id")
            if not user_id:
                raise EmbyAPIError("First user has no 'Id' field")
            self._user_id = user_id
            log.debug("Using Emby user ID: %s", self._user_id)
        return self._user_id

    def get_item(self, item_id: str) -> dict:
        """GET /Users/{userId}/Items/{id} — full item for round-trip update."""
        uid = self._get_user_id()
        result = self._request("GET", f"/Users/{uid}/Items/{item_id}")
        if result is None:
            raise EmbyAPIError(f"Empty response for GET /Users/{uid}/Items/{item_id}")
        return result

    def update_item(self, item_id: str, item_body: dict) -> None:
        """POST /Items/{id} — send full item body with modified fields."""
        self._request("POST", f"/Items/{item_id}", body=item_body)
        log.debug("Updated item %s", item_id)

    def list_genres(self) -> list[str]:
        """Return sorted list of all Audio genre names from Emby via GET /MusicGenres?Recursive=true."""
        result = self._request(
            "GET",
            "/MusicGenres?Recursive=true",
        )
        if result is None:
            log.warning("list_genres: Emby returned an empty response body")
            return []
        items = result.get("Items")
        if items is None:
            log.warning(
                "list_genres: Emby response missing 'Items' key; keys present: %s",
                list(result.keys()),
            )
            return []
        return sorted(
            (item.get("Name", "") for item in items if item.get("Name")),
            key=str.casefold,
        )


def _normalize_path(p: str) -> str:
    """Normalize a path for dict lookup, handling mixed separators."""
    return os.path.normcase(os.path.normpath(p)).replace("\\", "/")


# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------


def _path_parts(
    audio_path: Path | None, library_path: Path | None = None
) -> tuple[str, str]:
    """Best-effort fallback using the path relative to the library root."""
    if audio_path is None:
        return "", ""
    if library_path is not None:
        try:
            parts = audio_path.relative_to(library_path).parts
        except ValueError:
            parts = audio_path.parts
    else:
        parts = audio_path.parts
    # Artist/Album/Track (3+ segments) → (artist, album)
    if len(parts) >= 3:
        return parts[-3], parts[-2]
    # Album/Track (2 segments) → (empty, album)
    if len(parts) == 2:
        return "", parts[-2]
    return "", ""


def write_report(
    results: list[DetectionResult], path: Path, library_path: Path | None = None
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
                ]
            )
            for r in results:
                path_artist, path_album = _path_parts(r.audio_path, library_path)
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
                    ]
                )
    except OSError as exc:
        log.error("Cannot write report to %s: %s", path, exc)
        return
    log.info("Report written to %s", path)


# ---------------------------------------------------------------------------
# Orchestration
# ---------------------------------------------------------------------------


def process_library(config: Config) -> list[DetectionResult]:
    """Main flow: scan sidecars -> detect -> update Emby."""
    lp = config.library_path
    if not lp.exists():
        log.error("library_path does not exist: %s", lp)
        sys.exit(1)
    if not lp.is_dir():
        log.error("library_path is not a directory: %s", lp)
        sys.exit(1)
    pairs = scan_library(config.library_path)

    # Prefetch Emby items for path matching (even in dry-run, we read but don't write)
    emby: EmbyClient | None = None
    emby_items: dict[str, dict] = {}
    if config.emby_url and config.emby_api_key:
        emby = EmbyClient(config.emby_url, config.emby_api_key)
        try:
            emby_items = emby.prefetch_audio_items()
        except EmbyAPIError as exc:
            log.error("Failed to prefetch Emby items: %s", exc)
            log.error("Continuing in analysis-only mode")
            emby = None
    else:
        log.info("Emby URL or API key not configured; running in analysis-only mode")

    results: list[DetectionResult] = []
    sidecar_handled_paths: set[str] = set()

    for sidecar, audio in pairs:
        text = parse_sidecar(sidecar)
        tier, matched = classify_lyrics(text, config)

        dr = DetectionResult(
            sidecar_path=sidecar,
            audio_path=audio,
            tier=tier,
            matched_words=matched,
        )

        if tier:
            log.info("%s -> %s (words: %s)", sidecar.name, tier, ", ".join(matched))
        else:
            log.debug("%s -> clean", sidecar.name)

        if audio is None:
            dr.action = "no_audio_file"
            results.append(dr)
            continue

        # Resolve Emby item
        norm_audio = _normalize_path(str(audio))
        sidecar_handled_paths.add(norm_audio)
        emby_item = emby_items.get(norm_audio)
        if emby_item:
            dr.emby_item_id = emby_item.get("Id")
            dr.previous_rating = emby_item.get("OfficialRating", "") or ""
            dr.artist = emby_item.get("AlbumArtist", "") or ""
            dr.album = emby_item.get("Album", "") or ""

        # Decide action
        if tier is not None:
            # Explicit content found — set rating
            if emby is None:
                dr.action = "emby_unavailable"
            elif dr.emby_item_id is None:
                dr.action = "not_found_in_emby"
                log.warning("Audio file not found in Emby: %s", audio)
            elif config.dry_run:
                dr.action = "dry_run"
                log.info("[DRY RUN] Would set %s on %s", tier, audio.name)
            else:
                current_rating = (
                    emby_item.get("OfficialRating", "") if emby_item else ""
                )
                if current_rating == tier:
                    dr.action = "already_correct"
                    log.debug("Already rated %s: %s", tier, audio.name)
                else:
                    dr.action = _apply_rating(emby, dr.emby_item_id, tier, audio.name)
        elif config.clear:
            # Clean content + --clear flag — remove rating if set
            if emby is None:
                dr.action = "emby_unavailable"
            elif dr.emby_item_id is None:
                dr.action = "not_found_in_emby"
            elif config.dry_run:
                current_rating = (
                    emby_item.get("OfficialRating", "") if emby_item else ""
                )
                if current_rating:
                    dr.action = "dry_run_clear"
                    log.info("[DRY RUN] Would clear rating from %s", audio.name)
                else:
                    dr.action = "skipped"
            else:
                current_rating = (
                    emby_item.get("OfficialRating", "") if emby_item else ""
                )
                if current_rating:
                    dr.action = _apply_rating(emby, dr.emby_item_id, "", audio.name)
                    if dr.action == "set":
                        dr.action = "cleared"
                else:
                    dr.action = "skipped"
        else:
            dr.action = "skipped"

        results.append(dr)

    # --- Genre-based G rating pass ---
    if config.g_genres and emby is not None:
        lib_root = Path(_normalize_path(str(config.library_path.resolve())))
        for norm_path, item in emby_items.items():
            if norm_path in sidecar_handled_paths:
                continue
            if not Path(norm_path).is_relative_to(lib_root):
                continue
            matched_genre = match_g_genre(item, config.g_genres)
            if matched_genre is None:
                continue
            current_rating = item.get("OfficialRating", "") or ""
            item_id = item.get("Id", "")
            if not item_id:
                log.warning(
                    "Genre-pass: Emby item at %s has no 'Id' field; skipping", norm_path
                )
                continue
            dr = DetectionResult(
                sidecar_path=None,
                audio_path=Path(norm_path),
                tier="G",
                matched_words=[matched_genre],
                emby_item_id=item_id,
                previous_rating=current_rating,
                artist=item.get("AlbumArtist", "") or "",
                album=item.get("Album", "") or "",
            )
            if config.dry_run:
                dr.action = "dry_run_g_genre"
                log.info(
                    "[DRY RUN] Would set G on %s (genre: %s)", norm_path, matched_genre
                )
            elif current_rating == "G":
                dr.action = "g_genre_already_correct"
            else:
                dr.action = _apply_rating(emby, item_id, "G", norm_path)
                if dr.action == "set":
                    dr.action = "g_genre"
            results.append(dr)

    return results


def force_rate_library(config: Config) -> list[DetectionResult]:
    """--force-rating mode: set a fixed rating on ALL audio tracks under the
    library path, skipping tracks already at the target rating."""
    if not config.emby_url or not config.emby_api_key:
        log.error(
            "--force-rating requires an Emby URL and API key "
            "(via --emby-url/EMBY_URL and --emby-api-key/EMBY_API_KEY, .env, or TOML config)"
        )
        sys.exit(1)

    target = config.force_rating
    emby = EmbyClient(config.emby_url, config.emby_api_key)
    try:
        all_items = emby.prefetch_audio_items()
    except EmbyAPIError as exc:
        log.error("Failed to prefetch Emby items: %s", exc)
        sys.exit(1)

    # Filter to items under the library path (path-aware, avoids /music matching /music2)
    lib_root = Path(_normalize_path(str(config.library_path)))
    items_in_scope = {
        path: item
        for path, item in all_items.items()
        if Path(path).is_relative_to(lib_root)
    }
    log.info(
        "Force-rating: %d items under %s (of %d total)",
        len(items_in_scope),
        config.library_path,
        len(all_items),
    )

    results: list[DetectionResult] = []
    for norm_path, item in items_in_scope.items():
        item_id = item.get("Id", "")
        current = item.get("OfficialRating", "") or ""
        dr = DetectionResult(
            sidecar_path=None,
            audio_path=Path(norm_path),
            tier=target,
            emby_item_id=item_id,
            previous_rating=current,
            artist=item.get("AlbumArtist", "") or "",
            album=item.get("Album", "") or "",
        )
        if current == target:
            dr.action = "already_correct"
            log.debug("Already %s: %s", target, norm_path)
        elif config.dry_run:
            dr.action = "dry_run"
            log.info("[DRY RUN] Would set %s on %s", target, norm_path)
        else:
            dr.action = _apply_rating(emby, item_id, target, norm_path)
        results.append(dr)

    return results


def list_genres_mode(config: Config) -> None:
    """--list-genres mode: print all Audio genre names from Emby to stdout. Exits with non-zero status on error."""
    if not config.emby_url or not config.emby_api_key:
        print(
            "Error: --list-genres requires EMBY_URL and EMBY_API_KEY.",
            file=sys.stderr,
        )
        sys.exit(1)
    emby = EmbyClient(config.emby_url, config.emby_api_key)
    try:
        genres = emby.list_genres()
    except EmbyAPIError as exc:
        log.error("Failed to retrieve genres from Emby: %s", exc)
        sys.exit(1)
    print("=== Audio Genres ===")
    for g in genres:
        print(f"  {g}")
    if not genres:
        print("  (none found)")


def _apply_rating(
    emby: EmbyClient | None,
    item_id: str,
    rating: str,
    label: str,
) -> str:
    """GET-then-POST round-trip to set OfficialRating. Returns action string."""
    if emby is None:
        log.error(
            "_apply_rating called with no Emby client for %s (%s)", label, item_id
        )
        return "error"
    try:
        full_item = emby.get_item(item_id)
        full_item["OfficialRating"] = rating
        emby.update_item(item_id, full_item)
        verb = "Cleared rating from" if not rating else f"Set {rating} on"
        log.info("%s %s", verb, label)
        return "set"
    except EmbyAPIError as exc:
        log.error("Failed to update %s: %s", label, exc)
        return "error"


# ---------------------------------------------------------------------------
# CLI & Main
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="TagExplicitLyrics",
        description="Scan sidecar lyric files for explicit content and set "
        "OfficialRating on matching tracks via the Emby API.",
    )
    parser.add_argument(
        "library_path",
        nargs="?",
        default=None,
        help="Library root directory (overrides config)",
    )
    parser.add_argument(
        "--config",
        default=None,
        help="Path to TOML config file (default: explicit_config.toml next to script)",
    )
    parser.add_argument(
        "--env-file",
        default=None,
        help="Path to .env file (default: .env next to script; e.g. --env-file .env.prod)",
    )
    parser.add_argument(
        "--emby-url",
        default=None,
        help="Emby server URL (overrides config/.env)",
    )
    parser.add_argument(
        "--emby-api-key",
        default=None,
        help="Emby API key (overrides .env)",
    )
    parser.add_argument(
        "-n",
        "--dry-run",
        action="store_true",
        help="Analyze only — no Emby updates",
    )
    parser.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Debug logging",
    )
    parser.add_argument(
        "--report",
        default=None,
        help="CSV report output path",
    )
    parser.add_argument(
        "--clear",
        action="store_true",
        help="Clear ratings from tracks whose sidecars exist but contain no explicit words",
    )
    parser.add_argument(
        "--force-rating",
        default=None,
        metavar="RATING",
        help="Bypass detection; set this rating on ALL audio tracks in the library via Emby",
    )
    parser.add_argument(
        "--list-genres",
        action="store_true",
        help=(
            "Connect to Emby, print all Audio genre tags, then exit. "
            "Useful for populating [detection.g_genres] in the config."
        ),
    )
    return parser


def setup_logging(verbose: bool) -> None:
    level = logging.DEBUG if verbose else logging.INFO
    logging.basicConfig(
        level=level,
        format="%(asctime)s %(levelname)-8s %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )


def print_summary(results: list[DetectionResult]) -> None:
    sidecar_results = [
        r for r in results if r.sidecar_path is not None or r.action == "no_audio_file"
    ]
    genre_results = [
        r
        for r in results
        if r.action in ("g_genre", "g_genre_already_correct", "dry_run_g_genre")
    ]
    total = len(sidecar_results)
    r_count = sum(1 for r in sidecar_results if r.tier == "R")
    pg13_count = sum(1 for r in sidecar_results if r.tier == "PG-13")
    clean = sum(1 for r in sidecar_results if r.tier is None)
    audio_found = sum(
        1
        for r in sidecar_results
        if r.audio_path is not None and r.action != "no_audio_file"
    )
    emby_matched = sum(1 for r in sidecar_results if r.emby_item_id is not None)
    rated = sum(1 for r in results if r.action == "set")
    already = sum(1 for r in results if r.action == "already_correct")
    cleared = sum(1 for r in results if r.action == "cleared")
    dry = sum(1 for r in results if r.action.startswith("dry_run"))
    errors = sum(1 for r in results if r.action == "error")
    emby_unavail = sum(1 for r in results if r.action == "emby_unavailable")
    g_genre_rated = sum(1 for r in genre_results if r.action == "g_genre")
    g_genre_already = sum(
        1 for r in genre_results if r.action == "g_genre_already_correct"
    )

    print()
    print("=== Explicit Lyrics Scan Complete ===")
    print(f"  Sidecars scanned:    {total}")
    print(f"    R-rated:           {r_count}")
    print(f"    PG-13:             {pg13_count}")
    print(f"    Clean:             {clean}")
    print(f"  Audio files found:   {audio_found} / {total}")
    print(f"  Emby items matched:  {emby_matched} / {audio_found}")
    print(f"  Ratings set:         {rated}")
    print(f"  Already correct:     {already}")
    print(f"  Ratings cleared:     {cleared}")
    if g_genre_rated or g_genre_already:
        print(f"  G (genre-matched):   {g_genre_rated}")
        print(f"  Already G (genre):   {g_genre_already}")
    if emby_unavail:
        print(f"  Emby unavailable:    {emby_unavail}")
    if dry:
        print(f"  Dry-run would act:   {dry}")
    print(f"  Errors:              {errors}")


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()

    setup_logging(args.verbose)

    config = build_config(args)

    if args.list_genres:
        list_genres_mode(config)
        return

    if config.force_rating:
        results = force_rate_library(config)
    else:
        results = process_library(config)

    if config.report_path:
        write_report(results, config.report_path, config.library_path)

    print_summary(results)


if __name__ == "__main__":
    main()
