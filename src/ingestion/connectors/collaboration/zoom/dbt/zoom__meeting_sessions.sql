{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['zoom']
) }}

-- Zoom meeting-session stitching (issue #258) — extracted into its own
-- incremental staging model so that the downstream
-- `zoom__collab_meeting_activity` can stay `materialized='incremental'`
-- per DESIGN.md §3.7 (CR feedback on PR #284).
--
-- Grain: one row per `(tenant, source, uuid)`. Output column
-- `logical_meeting_id = concat(id, '#', cluster_idx)` collapses consecutive
-- sessions of the same Zoom Meeting ID whose end-of-prev → start-of-next
-- gap is ≤ `session_gap_seconds` (5 min) into a single logical meeting.
-- Host-drop rejoins (observed 11–147 s) thus map to the same logical
-- meeting; legitimate recurring meetings reusing the ID end up in
-- different clusters because their gaps are hours/days, not minutes.
--
-- Bounded rebuild window: bronze read is filtered to the last
-- `lookback_days` (90) of `_airbyte_extracted_at`. Anything older than
-- the window keeps the logical_meeting_id assigned at the time it was
-- written; the bound exceeds any plausible stitch interval (host-drop
-- gaps are seconds, never days) so we do not lose stitches at the
-- trailing edge.
--
-- `_version = now()` on every write means the ReplacingMergeTree always
-- keeps the most recent stitching decision per `uuid`. This is what lets
-- a late-arriving session re-cluster earlier sessions in the same chain:
-- the next incremental run rewrites every uuid in the 90-day window with
-- the new cluster_idx, and the higher `_version` wins on FINAL/argMax.
--
-- NULL `end_ts` handling: when a previous session in the chain has NULL
-- `end_ts` (Dashboard API has not yet flushed the close event), the
-- `dateDiff` over `lagInFrame(end_ts)` returns NULL. The CASE below
-- treats NULL gap as "new cluster", which conservatively SPLITS the chain
-- at that boundary. We intentionally do not fall back to
-- `lagInFrame(start_ts)` (a CodeRabbit suggestion on this PR): when
-- the previous session's end is unknown it may still be in progress,
-- and start-to-start could collapse two overlapping meetings. For
-- `meetings_attended`, over-splitting is the safer error than
-- over-merging — splitting may double-count one logical meeting,
-- merging would hide the second meeting entirely.

{% set session_gap_seconds = 300 %}
{% set lookback_days = 90 %}

WITH stitched AS (
    SELECT
        tenant_id,
        source_id,
        id,
        uuid,
        has_video,
        has_screen_share,
        sum(CASE WHEN gap_seconds IS NULL OR gap_seconds > {{ session_gap_seconds }} THEN 1 ELSE 0 END)
            OVER (
                PARTITION BY tenant_id, source_id, id
                ORDER BY start_ts
            ) AS cluster_idx
    FROM (
        SELECT
            *,
            dateDiff(
                'second',
                lagInFrame(end_ts) OVER (
                    PARTITION BY tenant_id, source_id, id
                    ORDER BY start_ts
                ),
                start_ts
            ) AS gap_seconds
        FROM (
            SELECT
                tenant_id,
                source_id,
                toString(id)                                            AS id,
                uuid,
                parseDateTimeBestEffortOrNull(coalesce(start_time, '')) AS start_ts,
                parseDateTimeBestEffortOrNull(coalesce(end_time, ''))   AS end_ts,
                has_video,
                has_screen_share
            FROM (
                SELECT *
                FROM {{ source('bronze_zoom', 'meetings') }}
                WHERE id IS NOT NULL
                  AND uuid IS NOT NULL AND uuid != ''
                  AND _airbyte_extracted_at >= now() - INTERVAL {{ lookback_days }} DAY
                ORDER BY _airbyte_extracted_at DESC
                LIMIT 1 BY uuid
            ) AS deduped
        ) AS with_dates
        WHERE start_ts IS NOT NULL
    ) AS with_gap
)

SELECT
    concat(tenant_id, '-', source_id, '-', uuid)                AS unique_key,
    tenant_id,
    source_id,
    uuid,
    id,
    concat(id, '#', toString(cluster_idx))                      AS logical_meeting_id,
    has_video,
    has_screen_share,
    now()                                                       AS collected_at,
    toUnixTimestamp64Milli(now64())                             AS _version
FROM stitched
