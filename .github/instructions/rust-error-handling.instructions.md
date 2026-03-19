---
applyTo: "smpr/src/**/*.rs"
excludeAgent: "coding-agent"
---

# Error Handling Review

Check for:
- `?` operator that silently skips items or returns early without logging
- Auth failures (401/403) masked as generic HTTP errors -- these should be
  surfaced distinctly so users know their API key is wrong
- Error body truncation: if some error paths truncate response bodies and
  others do not, flag the inconsistency
- `unwrap()` or `expect()` in non-test code -- should use `?` or match
- Functions returning `Ok(None)` that could mask real failures (e.g.,
  missing items silently skipped instead of warned about)
- IsExternal field defaults: when a field is missing from JSON, verify the
  default matches the API's actual behavior
