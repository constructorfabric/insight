---
status: accepted
date: 2026-07-25
supersedes: 0001-cursor-granularity-boundary-fix.md
---

# ADR-0002: `cursor_granularity: P0D` for exclusive, day-aligned `ending_at` on the usage/cost streams

**ID**: `cpt-insightspec-adr-claude-admin-cost-report-boundary`

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

[ADR-0001](0001-cursor-granularity-boundary-fix.md) set `cursor_granularity: PT1S` on the three incremental streams (`claude_admin_messages_usage`, `claude_admin_cost_report`, `claude_admin_code_usage`) to avoid an empty `starting_at == ending_at` window. That fix was incomplete for the two streams that inject `ending_at` (`messages_usage`, `cost_report`): a live staging sync (`docs/components/connectors/testing/claude-admin-sync-log.md`) showed `claude_admin_cost_report` returning **HTTP 400 `Invalid date range: ending date must be after starting date`** on **every** daily slice — 0 rows ingested.

The Anthropic Admin usage/cost API contract (confirmed from
`https://platform.claude.com/docs/en/api/admin-api/usage-cost/get-cost-report`
and `.../get-messages-usage-report`):

- `starting_at` is **required** and is snapped to the **start of the UTC day** for `bucket_width=1d`.
- `ending_at` is **optional and EXCLUSIVE** — "time buckets that **end before** this timestamp".
- Each returned bucket carries `starting_at` (inclusive) and `ending_at` (exclusive); the documented example is a full midnight→midnight day: `starting_at: 2025-08-01T00:00:00Z`, `ending_at: 2025-08-02T00:00:00Z`.

Airbyte CDK's `DatetimeBasedCursor` computes each slice as
`ending_at = (partition_start + step) − cursor_granularity` (verified in
`airbyte_cdk 6.60.16`, `datetime_based_cursor.py:270`, `_partition_daterange`).
With `step: P1D`, `cursor_granularity: PT1S` this yields `00:00:00Z → 23:59:59Z`.
The only 1d bucket `[00:00, next-00:00)` ends at the **next midnight**, which is
**not before** `23:59:59Z`, so **zero buckets qualify** — the API rejects it. The
sibling `messages_usage` endpoint tolerates the sub-day `ending_at` (returns
nothing useful) while `cost_report` hard-400s; the underlying defect is identical.

## Decision Drivers

- `ending_at` is exclusive and `starting_at` snaps to day start → a
  `midnight → next-midnight` window selects exactly one daily bucket.
- CDK couples `step` and `cursor_granularity`: defining one without the other is
  a validation error (`datetime_based_cursor.py:80-85`). `cursor_granularity`
  cannot simply be dropped while `step` remains.
- The cursor advances by `step` each iteration (`start = next_start`,
  `datetime_based_cursor.py:280`) **independent of** `cursor_granularity`, so
  `step: P1D` keeps one-day-per-slice advancement with no skipped days.
- Per-row `date`/`unique` are derived from `stream_interval['start_time'][:10]`
  (one day per slice) and the dbt Silver models
  (`dbt/claude_admin__ai_api_usage.sql`, `dbt/claude_admin__ai_dev_usage.sql`)
  aggregate one row per day — the fix must keep exactly one bucket per slice.

## Considered Options

1. **`cursor_granularity: P0D`** (`PT0S` is equivalent) — keep `step: P1D`,
   subtract zero → `ending_at = starting_at + P1D` = next midnight.
2. **Widen `step` to `P7D`/`P30D`** — fewer requests, but each returns multiple
   daily buckets all stamped with the slice-start `date` → corrupts Silver.
3. **Drop `end_time_option`** — send only `starting_at`. The usage/cost
   endpoints default to a multi-day window when `ending_at` is omitted
   (`messages_usage` `1d` default = 7 days), pulling multiple buckets per slice —
   same corruption as option 2, plus dependence on an undocumented default.

## Decision Outcome

**Chosen option: Option 1 — `cursor_granularity: P0D` on `messages_usage` and
`cost_report`.** `_parse_timedelta("P0D")` → `timedelta(0)`
(`datetime_based_cursor.py:319`), so:

```text
ending_at = (partition_start + P1D) − P0D = next UTC midnight
```

which is exclusive-correct: it selects exactly the one daily bucket
`[start_midnight, next_midnight)`.

`claude_admin_code_usage` is left on `cursor_granularity: PT1S` because it does
**not** inject `end_time_option` (sends only `starting_at`) — the boundary defect
never applied there, and its cursor uses the bare `%Y-%m-%d` format. No change.

### Consequences

- `cost_report` and `messages_usage` reach COMPLETE with rows > 0 and zero 400s.
- Exactly one bucket per daily slice — no gap, no double-count (verified against
  the exclusive contract), so the one-row-per-day dbt Silver models are correct.
- Historical and current-day partitions both produce a valid non-empty window
  (`start → start + P1D`); the terminal boundary slice where `start == end` is
  benign (the PT1S version produced the same tail slice).

### Confirmation

- `connector.yaml` carries `cursor_granularity: P0D` on `claude_admin_messages_usage`
  and `claude_admin_cost_report`; `claude_admin_code_usage` remains `PT1S`.
- L1 mock tests (`tests/test_claude_admin_cost_report.py`,
  `tests/test_claude_admin_messages_usage.py`) freeze the clock and assert the
  emitted `ending_at` query param equals `starting_at + P1D` (next midnight),
  never `23:59:59Z`.
- L2 live staging smoke confirms rows > 0, zero 400s, and one row per day (no
  duplicate days) in `bronze_claude_admin`.

## Pros and Cons of the Options

### Option 1: cursor_granularity: P0D

- Good — minimal, one value changes per stream; keeps explicit windowing.
- Good — matches the exclusive-`ending_at` contract exactly; one bucket per slice.
- Good — no change to `step`, so per-day `date`/`unique` and Silver stay correct.
- Neutral — relies on CDK's documented `(start + step) − granularity` arithmetic,
  pinned and covered by the L1 test.

### Option 2: Widen step (P7D/P30D)

- Good — cuts request volume ~7–30×.
- Bad — returns multiple daily buckets per request stamped with the slice-start
  date → corrupts `claude_admin__ai_api_usage` / `claude_admin__ai_dev_usage`
  unless the `date`/`unique` derivation is reworked to read each bucket's own
  `starting_at`. Out of scope for a boundary fix.

### Option 3: Drop end_time_option

- Good — sidesteps the boundary calculation.
- Bad — the API defaults to a multi-day window when `ending_at` is omitted →
  same multi-bucket corruption as option 2, plus reliance on an undocumented
  default.

## More Information

- Airbyte CDK `DatetimeBasedCursor` (6.60.x): slice end
  `= (partition_start + step) − cursor_granularity`; cursor advances by `step`;
  `step`/`cursor_granularity` must be defined together.
- Request-volume reduction (BUG-2 / #1902) is handled separately via
  `concurrency_level: 1` + a `Retry-After`-aware error handler, not by widening
  the step.

## Traceability

| Artifact | Requirement ID | Relationship |
|----------|---------------|--------------|
| [ADR-0001](0001-cursor-granularity-boundary-fix.md) | `cpt-insightspec-adr-claude-admin-cursor-granularity` | Supersedes — corrects the cost/usage-stream boundary |
| [DESIGN.md](../DESIGN.md) | `cpt-insightspec-constraint-claude-admin-date-range` | Satisfies — prevents the `Invalid date range` API rejection |
