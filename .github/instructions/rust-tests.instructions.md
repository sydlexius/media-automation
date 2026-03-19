---
applyTo: "smpr/src/**/*test*"
excludeAgent: "coding-agent"
---

# Test Code Review

- Tests run with `--test-threads=1` because config tests mutate process-global
  env vars. Verify test isolation where env vars are set and cleared.
- UAT tests are gated by `SMPR_UAT_TEST=1` env var with early return (not
  `#[ignore]`). Verify they print a skip message and return early when unset,
  rather than silently passing.
- Check test assertions for specificity: assert concrete values, not just
  `is_ok()` or `is_some()`.
- Verify that error paths are tested, not just success paths.
- Check for non-deterministic test behavior (unstable sort, timing-dependent).
