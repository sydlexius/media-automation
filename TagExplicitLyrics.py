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

AUDIO_EXTENSIONS = frozenset(
    {
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
    }
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
    "shitake",
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

    def __post_init__(self) -> None:
        self._r_exact_patterns = _compile_exact_patterns(self.r_exact)
        self._pg13_exact_patterns = _compile_exact_patterns(self.pg13_exact)


@dataclass
class DetectionResult:
    sidecar_path: Path
    audio_path: Path | None
    tier: str | None  # "R", "PG-13", or None (clean)
    matched_words: list[str] = field(default_factory=list)
    emby_item_id: str | None = None
    action: str = ""  # set | cleared | skipped | already_correct | not_found_in_emby |
    #                    error | no_audio_file | dry_run | dry_run_clear
    previous_rating: str = ""


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------


def load_env(path: Path) -> dict[str, str]:
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
    """Merge config layers: defaults -> TOML -> .env -> os.environ -> CLI."""
    script_dir = Path(__file__).resolve().parent
    toml_path = (
        Path(args.config) if args.config else script_dir / "explicit_config.toml"
    )
    toml = load_toml_config(toml_path)

    env_file = load_env(script_dir / ".env")

    # --- library_path ---
    library_path_str = (
        args.library_path
        or os.environ.get("LIBRARY_PATH")
        or env_file.get("LIBRARY_PATH")
        or toml.get("general", {}).get("library_path")
    )
    if not library_path_str:
        print(
            "Error: library_path is required (positional arg, env var, or config)",
            file=sys.stderr,
        )
        sys.exit(1)
    library_path = Path(library_path_str)

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
                # Check false positives bidirectionally
                is_fp = any(word in fp or fp in word for fp in fp_lower)
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
        return "R", r_stem_hits + r_exact_hits

    # Then PG-13
    pg13_stem_hits = detect_stems(
        word_tokens, config.pg13_stems, config.false_positives
    )
    pg13_exact_hits = detect_exact(text, config._pg13_exact_patterns)
    if pg13_stem_hits or pg13_exact_hits:
        return "PG-13", pg13_stem_hits + pg13_exact_hits

    return None, []


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
                    return json.loads(resp_data)
                return None
        except urllib.error.HTTPError as exc:
            body_snippet = ""
            try:
                body_snippet = exc.read().decode("utf-8", errors="replace")[:1024]
            except Exception:
                pass
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
                f"&Fields=Path,OfficialRating"
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
            start_index += page_size
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
            self._user_id = users[0]["Id"]
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


def _normalize_path(p: str) -> str:
    """Normalize a path for cross-platform dict lookup."""
    return os.path.normcase(os.path.normpath(p))


# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------


def _path_parts(audio_path: Path | None) -> tuple[str, str]:
    """Extract artist and album from typical Artist/Album/Track path layout."""
    if audio_path is None:
        return "", ""
    parts = audio_path.parts
    if len(parts) >= 3:
        return parts[-3], parts[-2]
    if len(parts) >= 2:
        return parts[-2], ""
    return "", ""


def write_report(results: list[DetectionResult], path: Path) -> None:
    """Write detection results to a CSV file."""
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
            artist, album = _path_parts(r.audio_path)
            track = r.audio_path.name if r.audio_path else r.sidecar_path.name
            writer.writerow(
                [
                    artist,
                    album,
                    track,
                    str(r.sidecar_path),
                    r.tier or "",
                    "; ".join(r.matched_words),
                    r.previous_rating,
                    r.action,
                ]
            )
    log.info("Report written to %s", path)


# ---------------------------------------------------------------------------
# Orchestration
# ---------------------------------------------------------------------------


def process_library(config: Config) -> list[DetectionResult]:
    """Main flow: scan sidecars -> detect -> update Emby."""
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

    results: list[DetectionResult] = []

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
        emby_item = emby_items.get(norm_audio)
        if emby_item:
            dr.emby_item_id = emby_item.get("Id")
            dr.previous_rating = emby_item.get("OfficialRating", "") or ""

        # Decide action
        if tier is not None:
            # Explicit content found — set rating
            if dr.emby_item_id is None:
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
            if dr.emby_item_id is None:
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

    return results


def force_rate_library(config: Config) -> list[DetectionResult]:
    """--force-rating mode: set a fixed rating on ALL audio tracks under the
    library path, skipping tracks already at the target rating."""
    if not config.emby_url or not config.emby_api_key:
        log.error("--force-rating requires --emby-url and EMBY_API_KEY")
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
            sidecar_path=Path(norm_path),
            audio_path=Path(norm_path),
            tier=target,
            emby_item_id=item_id,
            previous_rating=current,
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


def _apply_rating(
    emby: EmbyClient | None,
    item_id: str,
    rating: str,
    label: str,
) -> str:
    """GET-then-POST round-trip to set OfficialRating. Returns action string."""
    if emby is None:
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
    return parser


def setup_logging(verbose: bool) -> None:
    level = logging.DEBUG if verbose else logging.INFO
    logging.basicConfig(
        level=level,
        format="%(asctime)s %(levelname)-8s %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )


def print_summary(results: list[DetectionResult]) -> None:
    total = len(results)
    r_count = sum(1 for r in results if r.tier == "R")
    pg13_count = sum(1 for r in results if r.tier == "PG-13")
    clean = sum(1 for r in results if r.tier is None)
    audio_found = sum(
        1 for r in results if r.audio_path is not None and r.action != "no_audio_file"
    )
    emby_matched = sum(1 for r in results if r.emby_item_id is not None)
    rated = sum(1 for r in results if r.action == "set")
    already = sum(1 for r in results if r.action == "already_correct")
    cleared = sum(1 for r in results if r.action == "cleared")
    dry = sum(1 for r in results if r.action.startswith("dry_run"))
    errors = sum(1 for r in results if r.action == "error")

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
    if dry:
        print(f"  Dry-run would act:   {dry}")
    print(f"  Errors:              {errors}")


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()

    setup_logging(args.verbose)

    config = build_config(args)

    if config.force_rating:
        results = force_rate_library(config)
    else:
        results = process_library(config)

    if config.report_path:
        write_report(results, config.report_path)

    print_summary(results)


if __name__ == "__main__":
    main()
