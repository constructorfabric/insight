---
status: accepted
date: 2026-03-25
decision_makers:
  - Gregory Sotnichenko
---

# ADR-0001: Usage Events Deduplication Key — Simple Concatenation over Hash

## Context

The `cursor_usage_events` and `cursor_usage_events_daily_resync` streams need a deduplication key (`unique`) to enable idempotent upserts via ClickHouse `ReplacingMergeTree`. Two approaches were considered:

1. **Simple concatenation**: `unique = userEmail + timestamp` (epoch ms)
2. **SHA-256 hash**: `unique = SHA-256(timestamp|userEmail|model|kind|inputTokens|outputTokens)` (as used in the production Node.js system)

A potential collision exists with approach (1): if a single user generates two distinct events at the same millisecond, both records produce the same `unique` value. `ReplacingMergeTree` retains only the latest version, causing silent data loss of the other event.

## Decision

Use simple concatenation (`userEmail + timestamp`).

## Rationale

- **Collision probability is negligible.** The `timestamp` field is a server-side epoch ms value. Two distinct AI invocations from the same user resolving to the same server-side millisecond is theoretically possible but not observed in production data.
- **Production parity.** The existing production system (`usage-events-sync.ts`) uses the same formula: `unique: \`${ev.userEmail}${ev.timestamp}\``. No collision-related data loss has been reported.
- **Hash is not viable in the declarative manifest.** The SHA-256 approach requires `tokenUsage` fields (`inputTokens`, `outputTokens`) which are `null` for some events. A hash that includes nullable fields produces inconsistent keys for the same logical event across hourly and daily resync runs, breaking cross-stream deduplication at the Silver layer.
- **Cross-stream deduplication depends on key stability.** The hourly stream (`cursor_usage_events`) and daily resync stream (`cursor_usage_events_daily_resync`) may return the same event with different cost values (retroactive adjustments). The Silver dbt model deduplicates by `unique` — the key must be identical across both streams for the same logical event. Adding extra discriminators (e.g., `model`, `kind`) would not break this, but adds complexity without observed benefit.
- **Silver layer can re-hash if needed.** If collision becomes a problem at scale, the Silver dbt model can compute a stronger composite key from all available fields, without changing the Bronze connector.

## Consequences

- Accepted risk: if two events from the same user share an identical server-side millisecond timestamp, one will be silently lost at the Bronze level.
- Monitoring: no automated detection of lost events. If collision is suspected, compare Bronze record count against Cursor dashboard totals for the same period.
- Reversibility: changing the key formula requires a full resync of the affected streams (incremental state reset), since existing records have the old key format.
