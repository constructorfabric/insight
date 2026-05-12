-- depends_on: {{ ref('zoom__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
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
    coalesce(p.user_name, '') AS user_name,
    p.email AS email,
    lower(p.email) AS person_key,
    toDate(parseDateTimeBestEffort(p.join_time)) AS date,
    CAST(NULL AS Nullable(Int64)) AS calls_count,
    CAST(NULL AS Nullable(Int64)) AS meetings_organized,
    toInt64(count(*)) AS meetings_attended,
    CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_organized,
    CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_attended,
    CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_organized,
    CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_attended,
    toInt64(sum(
        if(p.join_time IS NOT NULL AND p.leave_time IS NOT NULL,
           dateDiff('second', parseDateTimeBestEffort(p.join_time), parseDateTimeBestEffort(p.leave_time)),
           0)
    )) AS audio_duration_seconds,
    toInt64(sumIf(
        if(p.join_time IS NOT NULL AND p.leave_time IS NOT NULL,
           dateDiff('second', parseDateTimeBestEffort(p.join_time), parseDateTimeBestEffort(p.leave_time)),
           0),
        m.has_video = true
    )) AS video_duration_seconds,
    toInt64(sumIf(
        if(p.join_time IS NOT NULL AND p.leave_time IS NOT NULL,
           dateDiff('second', parseDateTimeBestEffort(p.join_time), parseDateTimeBestEffort(p.leave_time)),
           0),
        m.has_screen_share = true
    )) AS screen_share_duration_seconds,
    CAST(NULL AS Nullable(String)) AS report_period,
    now() AS collected_at,
    'insight_zoom' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM {{ source('bronze_zoom', 'participants') }} p
LEFT JOIN {{ source('bronze_zoom', 'meetings') }} m
    ON p.meeting_uuid = m.uuid
    AND p.tenant_id = m.tenant_id
    AND p.source_id = m.source_id
WHERE p.join_time IS NOT NULL
  AND p.email IS NOT NULL
  AND p.email != ''
{% if is_incremental() %}
  AND (
    (SELECT max(date) FROM {{ this }}) IS NULL
    OR toDate(parseDateTimeBestEffort(p.join_time)) > (SELECT max(date) - INTERVAL 3 DAY FROM {{ this }})
  )
{% endif %}
GROUP BY
    p.tenant_id,
    p.source_id,
    p.email,
    p.user_name,
    toDate(parseDateTimeBestEffort(p.join_time))
