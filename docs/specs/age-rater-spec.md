# Spec: Comic Age Rating Backfill Tool

## Problem

A comic library of ~5,500 CBZ files was tagged with Metron-tagger, but only 43%
(2,436 files) received age ratings. ~3,100 files still have no usable age rating.
Age ratings can vary per issue within a series, so each issue needs its own lookup.
The user needs comprehensive per-issue age rating coverage to enable per-user age
restrictions in Komga.

## Goal

Build a Python CLI tool that backfills the `AgeRating` field in ComicInfo.xml
inside CBZ archives using a tiered approach:

1. **Amazon fetch** (via FlareSolverr) — search Amazon for each comic issue,
   scrape the age/content rating from the product detail page
2. **Publisher/imprint heuristic** — fall back to rule-based rating from a YAML
   config if Amazon can't match or doesn't have a rating
3. **Never overwrite** — skip files that already have a valid age rating

## Amazon Age Rating Scraping via FlareSolverr

### Why FlareSolverr

Amazon aggressively blocks automated requests with CAPTCHAs and bot detection.
The user has a FlareSolverr instance already running on their Unraid server.
FlareSolverr uses a headless browser to solve challenges and return clean HTML,
making Amazon scraping far more reliable than raw `requests`.

### FlareSolverr integration

FlareSolverr exposes a simple HTTP API:

```python
import requests

response = requests.post("http://<flaresolverr-host>:8191/v1", json={
    "cmd": "request.get",
    "url": "https://www.amazon.com/s?k=batman+1+dc+comics&i=digital-text",
    "maxTimeout": 60000
})

data = response.json()
html = data["solution"]["response"]  # Full rendered HTML
```

The tool should accept a `--flaresolverr` flag for the FlareSolverr base URL.
Default: `http://localhost:8191`.

### Search strategy

For each unrated file, extract from ComicInfo.xml:

- `Series` name
- `Number` (issue number)
- `Publisher` (to narrow results)
- `Volume` / `Year` (to disambiguate reboots)

Construct an Amazon search query:

```text
{series} #{number} {publisher}
```

Search URL:

```text
https://www.amazon.com/s?k={url_encoded_query}&i=digital-text&rh=n:156104011
```

Where `n:156104011` is the "Comics & Graphic Novels" Kindle Store category node.

Parse the search results HTML to find the best match, then fetch the product
detail page and extract the age rating.

### Product page age rating extraction

Amazon displays age/content ratings in the "Product details" or "Book details"
section of Kindle comic pages. Look for:

- A "Content Rating" or "Age Range" field in the product details table
- The former Comixology rating labels (e.g., "9+", "13+", "17+")
- A "Reading age" field (common on graphic novels)
- A "Best Sellers Rank" breadcrumb that indicates category (e.g., "Teen & Young
  Adult" vs "Adult")

Since Amazon's HTML structure changes without notice, the extraction logic should
be modular — isolate the HTML parsing into a single function that can be updated
when Amazon breaks it.

### Amazon → ComicInfo.xml rating mapping

| Amazon Rating | ComicInfo.xml AgeRating |
|---|---|
| All Ages | Everyone |
| 9+ | Everyone 10+ |
| 12+ | Teen |
| 13+ | Teen |
| 15+ | MA15+ |
| 17+ | Mature 17+ |
| 18+ | Adults Only 18+ |
| Reading age: 5-8 | Everyone |
| Reading age: 8-12 | Everyone 10+ |
| Reading age: 12+ | Teen |
| Reading age: 15+ | MA15+ |
| Reading age: 17+ | Mature 17+ |
| No rating found | (fall through to heuristic) |

### Match confidence

Amazon search results are noisy. To avoid mismatches:

- The result title must contain the series name (fuzzy match, case-insensitive)
- Prefer results whose title also contains the issue number
- If `Volume` or `Year` is available, prefer results matching the year
- If no confident match, log it and fall through to heuristic
- Do NOT auto-select ambiguous matches

### Throttling

- Minimum delay between requests: configurable via `--delay` (default: 4 seconds)
- FlareSolverr handles CAPTCHAs internally, but excessive speed still risks IP
  blocks at the Amazon level
- Support `--max-requests` to cap total Amazon requests per session
- At 4 seconds/request, ~3,100 files ≈ 3.5 hours

## Publisher/Imprint Heuristic (Fallback)

For files that Amazon can't match, apply rules from a YAML config file.

### Rule priority (most specific first)

1. **Series match** — exact series name maps to a rating
2. **Imprint match** — publisher + imprint combination maps to a rating
3. **Genre match** — if Genre field contains certain keywords, map to a rating
   (multiple matches → most restrictive wins)
4. **Publisher default** — publisher alone maps to a default rating
5. **Global default** — if nothing else matches

### Sample rules.yaml

```yaml
# Series-level overrides (highest priority)
series:
  "Bone: Quest for the Spark": "Everyone"
  "Saga": "Mature 17+"
  "Maus": "Mature 17+"
  "The Walking Dead": "Mature 17+"

# Imprint-level rules
imprints:
  "DC Comics":
    "Vertigo": "Mature 17+"
    "DC Black Label": "Mature 17+"
    "DC Kids": "Everyone"
    "DC Ink": "Teen"
    "Johnny DC": "Everyone"
  "Marvel":
    "MAX": "Mature 17+"
    "Marvel Knights": "Teen"
    "Marvel Adventures": "Everyone"
  "Image Comics":
    "Skybound": "Teen"
    "Top Cow": "Teen"

# Genre-based rules (case-insensitive matching)
# Multiple matches → most restrictive wins
genres:
  "Adults Only 18+":
    - "Erotica"
    - "Pornographic"
  "Mature 17+":
    - "Horror"
    - "Gore"
    - "Explicit"
  "Teen":
    - "Violence"
    - "Superhero"
    - "Action"
  "Everyone 10+":
    - "Fantasy"
    - "Adventure"
    - "Humor"

# Publisher defaults (lowest priority)
publishers:
  "DC Comics": "Teen"
  "Marvel": "Teen"
  "Image Comics": "Mature 17+"
  "Dark Horse Comics": "Teen"
  "IDW Publishing": "Teen"
  "BOOM! Studios": "Teen"
  "Dynamite Entertainment": "Teen"
  "Valiant Entertainment": "Teen"
  "Oni Press": "Teen"
  "Archie Comics": "Everyone"
  "Fantagraphics Books": "Mature 17+"
  "Drawn and Quarterly": "Mature 17+"
  "Viz Media": "Teen"
  "Kodansha": "Teen"
  "Andrews McMeel Publishing": "Everyone"
  "Scholastic": "Everyone"
  "First Second": "Everyone 10+"
  "Papercutz": "Everyone"
  "Action Lab Entertainment": "Everyone 10+"

# If absolutely nothing matches
default: "Teen"
```

## ComicInfo.xml AgeRating Valid Values

Per the Anansi Project schema (v2.0/v2.1), ordered least to most restrictive:

```text
Unknown
Rating Pending
Early Childhood
Everyone
Everyone 10+
G
Kids to Adults
PG
Teen
MA15+
Mature 17+
M
R18+
Adults Only 18+
X18+
```

The tool must only write values from this list.

## CLI Interface

```text
usage: age_rater.py [-h] [--config CONFIG] [--dry-run] [--log LOG]
                    [--amazon | --no-amazon]
                    [--flaresolverr URL] [--max-requests MAX]
                    [--delay DELAY] [--discover]
                    [--skip-no-comicinfo] path

Backfill ComicInfo.xml AgeRating via Amazon scraping + publisher heuristics.

positional arguments:
  path                  Root directory to scan recursively

options:
  -h, --help            show this help message and exit
  --config CONFIG       Path to rules YAML (default: rules.yaml)
  --dry-run             Show what would change without modifying files
  --log LOG             Path to write the change log (default: stdout)
  --amazon              Enable Amazon scraping (default: on)
  --no-amazon           Disable Amazon scraping, use heuristics only
  --flaresolverr URL    FlareSolverr base URL (default: http://localhost:8191)
  --max-requests MAX    Max Amazon requests per session (default: unlimited)
  --delay DELAY         Seconds between Amazon requests (default: 4)
  --discover            Scan and report unrated file metadata, then exit
  --skip-no-comicinfo   Skip files without ComicInfo.xml
```

## Modes

### Discovery mode (`--discover`)

Scans all CBZ files without age ratings and reports:

```text
=== Publishers (unrated files only) ===
DC Comics                    482
Marvel                       391
Image Comics                 288
...

=== Imprints (unrated files only) ===
DC Comics / Vertigo          67
DC Comics / DC Black Label   23
Marvel / MAX                 14
...

=== Genres (unrated files only) ===
Superhero                    1204
Action                       987
Horror                       156
...

=== No ComicInfo.xml (710 files) ===
/staging/Unknown Publisher/SomeFolder/  (12 files)
...
```

Run this first to drive the rules.yaml creation.

### Dry run mode (`--dry-run`)

Evaluates every file, prints what would happen, modifies nothing:

```text
[AMAZON]    Teen (Amazon: 13+)                → /path/to/file.cbz
[HEURISTIC] Mature 17+ (imprint: Vertigo)     → /path/to/file.cbz
[SKIPPED]   Already rated: MA15+              → /path/to/file.cbz
[NO MATCH]  No Amazon result, no rule         → /path/to/file.cbz
```

### Normal mode

Per file:

1. Check if already rated → skip
2. Try Amazon via FlareSolverr (if `--amazon`) → apply if found
3. Try heuristic rules → apply if matched
4. Log as unmatched

### Summary output

```text
Total files:          5546
Already rated:        2436
Rated via Amazon:     2200
Rated via heuristic:  600
No match:             200
No ComicInfo.xml:     110
```

## Resumability

Maintain a state file (`progress.json`) tracking:

- Files already processed (path → outcome + rating applied)
- Amazon request count

On restart, skip already-processed files. This prevents re-querying Amazon for
files that were already handled or returned "no match."

## Technical Requirements

- Python 3.10+
- Dependencies: PyYAML, requests, beautifulsoup4, standard library
  (zipfile, xml.etree.ElementTree)
- CBZ files only (ZIP archives)
- Must preserve all existing ComicInfo.xml content — only add/modify AgeRating
- Must handle XML encoding correctly (UTF-8)
- Must not corrupt archives — write to temp file, then replace original
- Must run as uid 99/gid 100 (Unraid standard)
- Shebangs: `#!/usr/bin/env python3`
- Should be runnable from the existing metron-tagger Docker container
  (add requests, beautifulsoup4, PyYAML via pip or extend the Dockerfile)

## Deployment

```text
/mnt/vms/dockerappdata/metron-tagger/config/age-rater/
├── age_rater.py
├── rules.yaml        (user-curated heuristic rules)
└── progress.json     (auto-generated state file)
```

Invoked from the metron-tagger container:

```bash
# Discovery
docker compose -f /mnt/vms/dockerappdata/metron-tagger/docker-compose.yml \
  run --rm --entrypoint python3 metron-tagger \
  /config/age-rater/age_rater.py --discover /staging

# Dry run with Amazon via FlareSolverr
docker compose -f /mnt/vms/dockerappdata/metron-tagger/docker-compose.yml \
  run --rm --entrypoint python3 metron-tagger \
  /config/age-rater/age_rater.py \
  --dry-run \
  --flaresolverr http://<unraid-ip>:8191 \
  --config /config/age-rater/rules.yaml \
  /staging

# Full run
docker compose -f /mnt/vms/dockerappdata/metron-tagger/docker-compose.yml \
  run --rm --entrypoint python3 metron-tagger \
  /config/age-rater/age_rater.py \
  --flaresolverr http://<unraid-ip>:8191 \
  --config /config/age-rater/rules.yaml \
  /staging

# Heuristic only (no Amazon)
docker compose -f /mnt/vms/dockerappdata/metron-tagger/docker-compose.yml \
  run --rm --entrypoint python3 metron-tagger \
  /config/age-rater/age_rater.py \
  --no-amazon \
  --config /config/age-rater/rules.yaml \
  /staging
```

## Recommended Workflow

1. `--discover` — understand your unrated files
2. Curate `rules.yaml` based on the discovery output
3. `--dry-run` with Amazon — verify matching quality
4. Full run with Amazon — let it scrape what it can (~3.5 hours at 4s/req)
5. Full run with `--no-amazon` — heuristic fills remaining gaps
6. Review "no match" list and handle manually in Komga

## Edge Cases

- `<AgeRating>Unknown</AgeRating>` → treated as unrated
- `<AgeRating>Rating Pending</AgeRating>` → treated as unrated
- No `<AgeRating>` element → treated as unrated
- No ComicInfo.xml → create minimal one with AgeRating (unless `--skip-no-comicinfo`)
- Genre matching → case-insensitive
- Publisher/imprint matching → case-insensitive
- Series matching → exact, case-sensitive
- Genre multiple matches → most restrictive rating wins
- Interrupted run → resume from progress.json
- FlareSolverr unavailable → log error and fall through to heuristic for all files

## Amazon Scraping Caveats

Amazon scraping is inherently fragile. Amazon changes page structure without
notice. This tool is designed for personal, one-time use to backfill a private
library. The HTML parsing logic should be isolated into a single module so it
can be updated when Amazon inevitably changes their layout. FlareSolverr handles
bot detection but doesn't guarantee Amazon won't change the data structure of
product pages.
