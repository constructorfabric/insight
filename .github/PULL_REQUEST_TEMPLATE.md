## What & why

<!-- One paragraph. Link the FEATURE / issue this PR satisfies. -->

**Linked:** <!-- #issue / TAF-XXX -->

## Definition of Done

> **Red-then-green:** a fix lands with a test that **failed before** it and **passes after**. The expected value comes from the spec, never from what the code currently returns.

- [ ] Requirement linked & scope matches the FEATURE / issue
- [ ] Added/updated a test that is **red without this change, green with it**
- [ ] Tests are at the right layer (unit / contract / e2e) and assert **intended** behavior, not current output
- [ ] `make check` (fmt · lint · typecheck · tests · relevant gates) passes **locally** before push
- [ ] Coverage held or raised vs the base branch (the ratchet)
- [ ] Negative paths covered (null / empty / error / unauthorized) — no silent failure
- [ ] Docs / spec (DESIGN / FEATURE) updated

### Data (dbt / connectors / SQL) — or check N/A: [ ]

- [ ] New connector: its data actually reaches the unified `silver:class_*` set
- [ ] Changed silver schema: dependent **gold views still resolve** (no missing column)
- [ ] New/changed model: strict **`contract: {enforced: true}`** with declared `data_type` (prevents type drift)
- [ ] Changed a metric: added/updated a **golden-value** regression test (parity with prior values)
- [ ] ReplacingMergeTree `order_by` dedup keys **cannot be NULL** (NULL ≠ NULL → double counting)
- [ ] Source freshness covered where applicable

### API / Frontend (UI) — or check N/A: [ ]

- [ ] **Honest NULLs:** when there's no data the API returns `NULL` / a `ComingSoon` flag, never `0`
- [ ] **Render contract:** UI distinguishes empty state ("No data") from a server **error** (HTTP 500 → error/retry, not "No data")
- [ ] Component / e2e tests (Vitest / Playwright) cover the new or changed states

## Evidence

<!-- Paste the test summary + coverage % (before → after) and any gate output. -->

```text

```
