-- Bronze → staging: canonical Cursor per-event usage surface.
--
-- The Cursor connector ingests the same events twice (issue #261):
--   1. `cursor_usage_events`               — hourly incremental stream;
--      `chargedCents` may not yet be finalized (Cursor recomputes the
--      cost up to ~24h after the event).
--   2. `cursor_usage_events_daily_resync`  — re-fetches yesterday's
--      window with the finalized `chargedCents`.
--
-- Both write to separate Bronze tables, so the finalized values from
-- (2) never reach downstream models — and any consumer that UNIONs
-- both would double-count.
--
-- This staging model resolves the split: UNION ALL both tables, then
-- keep the latest row per `unique_key` ordered by
-- `_airbyte_extracted_at` DESC. On the overlap window (last ~10 days)
-- the resync row wins because it is emitted on a later sync; outside
-- the overlap (older history) the only row available is the original
-- main-stream row, so it is preserved.
--
-- Materialized as a view: per-event volumes are modest (~tens of
-- thousands of rows per tenant per month) and downstream consumers
-- are read-on-demand. Promote to incremental keyed on (date, userId)
-- once the bronze volume grows past ~10M rows per tenant.
--
-- Downstream contract: consumers of per-event Cursor usage MUST read
-- this view instead of the raw bronze tables.
--
-- The bronze `tokenUsage` column is typed `Nullable(JSON)` by Airbyte.
-- ClickHouse views cannot carry JSON columns ("storage View doesn't
-- support dynamic subcolumns"), so we cast to `Nullable(String)` —
-- downstream can parse with `JSONExtract*` if needed.

{{ config(
    materialized='view',
    schema='staging',
    tags=['cursor']
) }}

SELECT
    * EXCEPT tokenUsage,
    CAST(tokenUsage AS Nullable(String)) AS tokenUsage
FROM (
    SELECT
        *,
        'main' AS _origin
    FROM {{ source('bronze_cursor', 'cursor_usage_events') }}

    UNION ALL

    SELECT
        *,
        'resync' AS _origin
    FROM {{ source('bronze_cursor', 'cursor_usage_events_daily_resync') }}
) AS u
-- Tie-breaker on equal _airbyte_extracted_at: prefer the resync row so the
-- finalized chargedCents wins deterministically. Otherwise the main row is
-- already the only candidate (older history) or already loses on extraction
-- time (resync extracted later).
ORDER BY
    _airbyte_extracted_at DESC,
    (_origin = 'resync') DESC
LIMIT 1 BY unique_key
