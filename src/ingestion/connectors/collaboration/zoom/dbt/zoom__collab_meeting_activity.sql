-- depends_on: {{ ref('zoom__meeting_sessions') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['zoom', 'silver:class_collab_meeting_activity']
) }}

-- Zoom meeting activity aggregated per user per day.
--
-- Grain: (tenant, source, email, date). We intentionally filter out participants
-- without an email (guests / anonymous joiners) because:
--   1. Without a stable user identifier, a COALESCE(email, user_name) key is
--      unstable — the same person can flip keys between batches depending on
--      whether Zoom returns their email that run.
--   2. Anonymous participants can't be joined to identity at the Silver layer
--      anyway, so they add noise without enabling any downstream use case.
-- If Zoom ever starts exposing a stable participant_id/user_id, switch to that.
--
-- Meeting-session stitching (issue #258): the (tenant, source, uuid) →
-- logical_meeting_id mapping is computed in the upstream
-- `zoom__meeting_sessions` model (per CR on PR #284/#479, so this model
-- can stay `materialized='incremental'` per DESIGN.md §3.7). We join FINAL
-- to get the latest cluster assignment per uuid; `meetings_attended` then
-- counts distinct logical meetings instead of distinct sessions,
-- collapsing host-drop rejoins. See the header of zoom__meeting_sessions
-- for threshold rationale and NULL-end_ts handling.
--
-- Duration semantics (issue #263 — tradeoff):
--
-- M365's getTeamsUserActivityUserDetail report exposes true minute-of-video
-- and minute-of-screenshare per user per day. Zoom does NOT — no Dashboard /
-- Reports endpoint we can call here returns per-participant video duration.
-- The participant payload we have in bronze carries only "did this user use
-- video / share at all during the session" signals:
--
--   p.camera             Nullable(String)  -- camera device name. NULL/'' when
--                                             no camera was used (mic-only join).
--                                             Empirically ≈26% non-NULL — the
--                                             real "user had video on" signal.
--                                             Caveat: `p.video_connection_type`
--                                             is the network transport (Reliable
--                                             UDP / P2P / TCP / ...) and is
--                                             populated for ~99% of rows — it
--                                             is NOT a camera-on flag.
--   p.share_desktop      Nullable(Bool)
--   p.share_application  Nullable(Bool)
--   p.share_whiteboard   Nullable(Bool)
--
-- We use those signals to gate the session length, so a participant who
-- never turned on the camera contributes video_duration_seconds=0
-- (matching the Teams semantics directionally). We deliberately DO NOT
-- use the `has_video` / `has_screen_share` flags carried through
-- `zoom__meeting_sessions` — those are MEETING-level (any participant
-- ever turned on video) and would count the full session for every
-- attendee of a meeting where someone else had video.
--
-- Known limitation that this fix does NOT eliminate: if a Zoom
-- participant turned the camera on for one minute and then off for the
-- remaining 59, their `camera` device name is still populated for the
-- session and their full session length is attributed to
-- `video_duration_seconds`. Zoom rows therefore still OVER-ESTIMATE
-- video / screen-share duration vs the true minute-of-X numbers M365
-- produces. Cross-vendor aggregates that sum `video_duration_seconds`
-- across Zoom and M365 are NOT directly comparable.
--
-- True minute-of-video parity would require the Zoom Dashboard QoS
-- endpoint (`/metrics/meetings/{id}/participants/qos`) — paid tier,
-- separate stream, new rate-limit budget. Tracked as a follow-up to
-- #263.

SELECT
    p.tenant_id,
    p.source_id AS insight_source_id,
    MD5(concat(
        p.tenant_id, '-',
        p.source_id, '-',
        lower(p.email), '-',
        toString(toDate(parseDateTimeBestEffort(p.join_time)))
    )) AS unique_key,
    p.email AS user_id,
    -- Pick one display name when the same email surfaces under multiple
    -- spellings (e.g., "Karolis Dambrava" vs "karolisdambrava"). Without
    -- this, GROUP BY would split them and produce two rows with identical
    -- unique_key — the staging model's `unique_key` is keyed on
    -- (tenant, source, lower(email), date), so user_name is non-keying.
    coalesce(any(p.user_name), '') AS user_name,
    p.email AS email,
    lower(p.email) AS person_key,
    toDate(parseDateTimeBestEffort(p.join_time)) AS date,
    CAST(NULL AS Nullable(Int64)) AS calls_count,
    CAST(NULL AS Nullable(Int64)) AS meetings_organized,
    -- uniqExact over logical_meeting_id collapses host-drop rejoins into one.
    -- Falls back to participant.meeting_uuid when the JOIN misses (meeting
    -- row not yet stitched) — preserves "one row → one meeting" behavior
    -- consistent with the previous count(*) for unstitched data.
    toInt64(uniqExact(coalesce(ml.logical_meeting_id, p.meeting_uuid))) AS meetings_attended,
    CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_organized,
    CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_attended,
    CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_organized,
    CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_attended,
    toInt64(sum(
        if(p.join_time IS NOT NULL AND p.leave_time IS NOT NULL,
           dateDiff('second', parseDateTimeBestEffort(p.join_time), parseDateTimeBestEffort(p.leave_time)),
           0)
    )) AS audio_duration_seconds,
    -- #263: gate by per-participant `camera` device name (NULL/'' means
    -- the participant did not use a camera in this session), not by the
    -- meeting-level `has_video` flag from sessions. See header for the
    -- over-estimate caveat (any-video-ever-in-session counts the whole
    -- session).
    toInt64(sumIf(
        if(p.join_time IS NOT NULL AND p.leave_time IS NOT NULL,
           dateDiff('second', parseDateTimeBestEffort(p.join_time), parseDateTimeBestEffort(p.leave_time)),
           0),
        p.camera IS NOT NULL AND p.camera != ''
    )) AS video_duration_seconds,
    toInt64(sumIf(
        if(p.join_time IS NOT NULL AND p.leave_time IS NOT NULL,
           dateDiff('second', parseDateTimeBestEffort(p.join_time), parseDateTimeBestEffort(p.leave_time)),
           0),
        coalesce(p.share_desktop, false)
        OR coalesce(p.share_application, false)
        OR coalesce(p.share_whiteboard, false)
    )) AS screen_share_duration_seconds,
    CAST(NULL AS Nullable(String)) AS report_period,
    now() AS collected_at,
    'insight_zoom' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM (
    -- Drop bronze re-emit duplicates for participants. Bronze Airbyte
    -- re-emits produce multiple identical rows per (meeting_uuid,
    -- participant_uuid, join_time); without dedup, SUM(duration) is
    -- inflated by the re-emit factor.
    SELECT *
    FROM {{ source('bronze_zoom', 'participants') }}
    WHERE join_time IS NOT NULL
      AND email IS NOT NULL AND email != ''
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY meeting_uuid, participant_uuid, join_time
) AS p
LEFT JOIN {{ ref('zoom__meeting_sessions') }} FINAL AS ml
    ON p.meeting_uuid = ml.uuid
    AND p.tenant_id = ml.tenant_id
    AND p.source_id = ml.source_id
{% if is_incremental() %}
WHERE (
    (SELECT max(date) FROM {{ this }}) IS NULL
    OR toDate(parseDateTimeBestEffort(p.join_time)) > (SELECT max(date) - INTERVAL 3 DAY FROM {{ this }})
)
{% endif %}
GROUP BY
    p.tenant_id,
    p.source_id,
    p.email,
    toDate(parseDateTimeBestEffort(p.join_time))
