-- depends_on: {{ ref('zoom__meeting_sessions') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
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

{%- set participants = source('bronze_zoom', 'participants') %}
{%- set meetings = source('bronze_zoom', 'meetings') %}
WITH
{%- if is_incremental() %}
-- INVARIANT: the watermark is the source EXTRACT time, not the meeting date, because
-- re-pulled rows carry a fresh `_airbyte_extracted_at`. Company time depends on every
-- attendee, so a late attendee re-opens all dates of that meeting.
recently_extracted_meetings AS (
    SELECT DISTINCT meeting_uuid
    FROM {{ participants }}
    WHERE _airbyte_extracted_at
          > (SELECT max(_airbyte_extracted_at) FROM {{ participants }}) - INTERVAL 3 DAY
    UNION DISTINCT
    SELECT DISTINCT uuid AS meeting_uuid
    FROM {{ meetings }}
    WHERE _airbyte_extracted_at
          > (SELECT max(_airbyte_extracted_at) FROM {{ meetings }}) - INTERVAL 3 DAY
),

dates_to_rebuild AS (
    SELECT DISTINCT toDate(parseDateTimeBestEffortOrNull(join_time), 'UTC') AS date
    FROM {{ participants }}
    WHERE meeting_uuid IN (SELECT meeting_uuid FROM recently_extracted_meetings)
    UNION DISTINCT
    SELECT DISTINCT toDate(parseDateTimeBestEffortOrNull(start_time), 'UTC') AS date
    FROM {{ meetings }}
    WHERE uuid IN (SELECT meeting_uuid FROM recently_extracted_meetings)
),

meetings_in_scope AS (
    SELECT DISTINCT meeting_uuid
    FROM {{ participants }}
    WHERE toDate(parseDateTimeBestEffortOrNull(join_time), 'UTC') IN (SELECT date FROM dates_to_rebuild)
    UNION DISTINCT
    SELECT DISTINCT uuid AS meeting_uuid
    FROM {{ meetings }}
    WHERE toDate(parseDateTimeBestEffortOrNull(start_time), 'UTC') IN (SELECT date FROM dates_to_rebuild)
),
{%- endif %}

deduped_participants AS (
    SELECT
        tenant_id,
        source_id,
        meeting_uuid,
        participant_uuid,
        email,
        user_name,
        camera,
        share_desktop,
        share_application,
        share_whiteboard,
        parseDateTimeBestEffortOrNull(join_time) AS joined_at,
        parseDateTimeBestEffortOrNull(leave_time) AS left_at
    FROM {{ participants }}
    WHERE parseDateTimeBestEffortOrNull(join_time) IS NOT NULL
    {%- if is_incremental() %}
      AND (
          (SELECT count() FROM {{ this }}) = 0
          OR meeting_uuid IN (SELECT meeting_uuid FROM meetings_in_scope)
      )
    {%- endif %}
    -- WORKAROUND: Airbyte re-emits identical participant rows; keep the latest one.
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY meeting_uuid, participant_uuid, joined_at
),

attendance AS (
    SELECT
        tenant_id,
        source_id,
        meeting_uuid,
        if(
            email IS NOT NULL AND email != '',
            lower(email),
            concat('guest:', ifNull(participant_uuid, ''))
        ) AS attendee,
        min(joined_at) AS first_join,
        max(left_at) AS last_leave
    FROM deduped_participants
    GROUP BY tenant_id, source_id, meeting_uuid, attendee
),

companions AS (
    SELECT
        me.tenant_id,
        me.source_id,
        me.meeting_uuid,
        me.attendee,
        min(other.first_join) AS others_from,
        max(other.last_leave) AS others_until,
        count() AS others
    FROM attendance AS me
    INNER JOIN attendance AS other
        ON other.tenant_id = me.tenant_id
        AND other.source_id = me.source_id
        AND other.meeting_uuid = me.meeting_uuid
        AND other.attendee != me.attendee
    GROUP BY me.tenant_id, me.source_id, me.meeting_uuid, me.attendee
),

sessions_with_company AS (
    SELECT
        own.tenant_id,
        own.source_id,
        own.meeting_uuid,
        own.email,
        own.user_name,
        own.camera,
        own.share_desktop,
        own.share_application,
        own.share_whiteboard,
        own.joined_at,
        -- WORKAROUND: an unmatched LEFT JOIN gives NULL or 0 depending on join_use_nulls,
        -- and greatest()/least() skip NULL, so test the count before clamping.
        if(
            ifNull(c.others, 0) = 0,
            0,
            greatest(0, ifNull(dateDiff(
                'second',
                greatest(own.joined_at, c.others_from),
                least(own.left_at, c.others_until)
            ), 0))
        ) AS company_seconds
    FROM deduped_participants AS own
    LEFT JOIN companions AS c
        ON c.tenant_id = own.tenant_id
        AND c.source_id = own.source_id
        AND c.meeting_uuid = own.meeting_uuid
        AND c.attendee = lower(own.email)
    WHERE own.email IS NOT NULL AND own.email != ''
),

attended_days AS (
    SELECT
        p.tenant_id AS tenant_id,
        p.source_id AS source_id,
        lower(p.email) AS person_key,
        toDate(p.joined_at, 'UTC') AS date,
        min(p.email) AS attendee_email,
        coalesce(max(p.user_name), '') AS attendee_name,
        -- INVARIANT: logical_meeting_id folds host-drop rejoins into one meeting;
        -- meeting_uuid stands in until the meeting is stitched.
        toInt64(uniqExactIf(coalesce(ml.logical_meeting_id, p.meeting_uuid), p.company_seconds > 0)) AS meetings_attended,
        toInt64(sum(p.company_seconds)) AS audio_duration_seconds,
        -- INVARIANT: video is gated by the participant's own camera, not the meeting-level
        -- has_video flag, which would credit everyone in a meeting where anyone had video.
        toInt64(sumIf(p.company_seconds, p.camera IS NOT NULL AND p.camera != '')) AS video_duration_seconds,
        toInt64(sumIf(
            p.company_seconds,
            coalesce(p.share_desktop, false)
            OR coalesce(p.share_application, false)
            OR coalesce(p.share_whiteboard, false)
        )) AS screen_share_duration_seconds
    FROM sessions_with_company AS p
    LEFT JOIN {{ ref('zoom__meeting_sessions') }} AS ml FINAL
        ON p.meeting_uuid = ml.uuid
        AND p.tenant_id = ml.tenant_id
        AND p.source_id = ml.source_id
    GROUP BY tenant_id, source_id, person_key, date
),

hosted_sessions AS (
    SELECT
        tenant_id,
        source_id,
        uuid AS meeting_uuid,
        email AS host_email,
        parseDateTimeBestEffortOrNull(start_time) AS started_at
    FROM {{ meetings }}
    WHERE email IS NOT NULL AND email != ''
      AND uuid IS NOT NULL AND uuid != ''
      AND parseDateTimeBestEffortOrNull(start_time) IS NOT NULL
    {%- if is_incremental() %}
      AND (
          (SELECT count() FROM {{ this }}) = 0
          OR uuid IN (SELECT meeting_uuid FROM meetings_in_scope)
      )
    {%- endif %}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY tenant_id, source_id, uuid
),

attended_by_others AS (
    SELECT
        h.tenant_id AS tenant_id,
        h.source_id AS source_id,
        h.meeting_uuid AS meeting_uuid,
        h.host_email AS host_email,
        h.started_at AS started_at
    FROM hosted_sessions AS h
    INNER JOIN attendance AS a
        ON a.tenant_id = h.tenant_id
        AND a.source_id = h.source_id
        AND a.meeting_uuid = h.meeting_uuid
        AND a.attendee != lower(h.host_email)
    GROUP BY tenant_id, source_id, meeting_uuid, host_email, started_at
),

organized_days AS (
    -- INVARIANT: like attendance, a stitched meeting counts once per UTC day it ran,
    -- so organized and attended meetings stay comparable day by day.
    SELECT
        o.tenant_id AS tenant_id,
        o.source_id AS source_id,
        lower(o.host_email) AS person_key,
        toDate(o.started_at, 'UTC') AS date,
        min(o.host_email) AS organizer_email,
        toInt64(uniqExact(coalesce(ml.logical_meeting_id, o.meeting_uuid))) AS meetings_organized
    FROM attended_by_others AS o
    LEFT JOIN {{ ref('zoom__meeting_sessions') }} AS ml FINAL
        ON o.meeting_uuid = ml.uuid
        AND o.tenant_id = ml.tenant_id
        AND o.source_id = ml.source_id
    GROUP BY tenant_id, source_id, person_key, date
),

person_days AS (
    SELECT
        tenant_id,
        source_id,
        person_key,
        date,
        attendee_email                AS contact_email,
        attendee_name                 AS display_name,
        toUInt8(1)                    AS from_attendance,
        meetings_attended,
        audio_duration_seconds,
        video_duration_seconds,
        screen_share_duration_seconds,
        toInt64(0)                    AS meetings_organized
    FROM attended_days
    UNION ALL
    SELECT
        tenant_id,
        source_id,
        person_key,
        date,
        organizer_email               AS contact_email,
        ''                            AS display_name,
        toUInt8(0)                    AS from_attendance,
        toInt64(0)                    AS meetings_attended,
        toInt64(0)                    AS audio_duration_seconds,
        toInt64(0)                    AS video_duration_seconds,
        toInt64(0)                    AS screen_share_duration_seconds,
        meetings_organized
    FROM organized_days
)

SELECT
    tenant_id,
    source_id AS insight_source_id,
    MD5(concat(tenant_id, '-', source_id, '-', person_key, '-', toString(date))) AS unique_key,
    argMax(contact_email, (from_attendance, contact_email)) AS user_id,
    toNullable(max(display_name)) AS user_name,
    user_id AS email,
    person_key,
    date,
    CAST(NULL AS Nullable(Int64)) AS calls_count,
    toNullable(sum(meetings_organized)) AS meetings_organized,
    sum(meetings_attended) AS meetings_attended,
    CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_organized,
    CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_attended,
    CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_organized,
    CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_attended,
    toNullable(sum(audio_duration_seconds)) AS audio_duration_seconds,
    toNullable(sum(video_duration_seconds)) AS video_duration_seconds,
    toNullable(sum(screen_share_duration_seconds)) AS screen_share_duration_seconds,
    CAST(NULL AS Nullable(String)) AS report_period,
    now() AS collected_at,
    'insight_zoom' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM person_days
{%- if is_incremental() %}
WHERE (
    (SELECT count() FROM {{ this }}) = 0
    OR date IN (SELECT date FROM dates_to_rebuild)
)
{%- endif %}
GROUP BY tenant_id, source_id, person_key, date
