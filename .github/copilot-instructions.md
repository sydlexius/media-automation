# Copilot PR Review Instructions

## Project summary

`smpr` is a Rust CLI for Emby/Jellyfin parental rating management. Fetches
lyrics from media server APIs, detects explicit content using tiered word
detection (R / PG-13), and sets `OfficialRating` on matching audio tracks.

## Focus areas (review these carefully)

- Security vulnerabilities: command injection, path traversal, credential leakage
- Correctness bugs: logic errors, unwrap on Option/Result, silent error swallowing
- Error handling: `?` operator silently skipping items, auth failures masked as
  transient errors, error body truncation inconsistencies
- API correctness: URL encoding for user-provided IDs, header injection, response
  body handling
- Cross-platform issues: path separators, case sensitivity, hostname handling

## Rust conventions

- Error enums use hand-rolled `Display`/`Error` impls (no `thiserror` or `anyhow`)
- Prefer explicit error handling over `.unwrap()` in non-test code
- Prefer `?` with context via `.map_err()` and project-specific error types
- Test code uses `--test-threads=1` (config tests mutate process-global env vars)

## Files to skip

- `Cargo.lock` -- auto-generated dependency lockfile
- `*.csv` -- test output data
- `.env*` -- credential files (filtered by gitignore)

## Known patterns -- do not flag

- `#[ignore]` on UAT/integration tests is intentional (require live server)
- `dotenvy::from_path` errors are intentionally ignored for auto-discovered .env files
- `--test-threads=1` is required, not a performance concern

## Review priority order

Focus review effort in this order: security > correctness > completeness > style.
