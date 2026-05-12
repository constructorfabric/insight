-- depends_on: {{ ref('m365__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['m365', 'silver:class_collab_meeting_activity']
) }}

SELECT
    tenant_id,
    source_id AS insight_source_id,
    MD5(concat(tenant_id, '-', source_id, '-', coalesce(userPrincipalName, ''), '-', toString(reportRefreshDate))) AS unique_key,
    userPrincipalName AS user_id,
    userPrincipalName AS user_name,
    userPrincipalName AS email,
    if(userPrincipalName IS NOT NULL AND userPrincipalName != '',
       lower(userPrincipalName),
       '') AS person_key,
    toDate(reportRefreshDate) AS date,
    -- Cast counts and durations to Int64 to match zoom__collab_meeting_activity
    -- when both feeders UNION ALL into silver.class_collab_meeting_activity.
    -- Bronze stores them as Decimal(38, 9) / Float64; CH 25.3 refuses
    -- Int64 ∪ Decimal/Float with NO_COMMON_TYPE.
    toInt64(coalesce(callCount, 0)) AS calls_count,
    toInt64(coalesce(meetingsOrganizedCount, 0)) AS meetings_organized,
    toInt64(coalesce(meetingsAttendedCount, 0)) AS meetings_attended,
    toInt64(coalesce(adHocMeetingsOrganizedCount, 0)) AS adhoc_meetings_organized,
    toInt64(coalesce(adHocMeetingsAttendedCount, 0)) AS adhoc_meetings_attended,
    toInt64(COALESCE(scheduledOneTimeMeetingsOrganizedCount, 0)
        + COALESCE(scheduledRecurringMeetingsOrganizedCount, 0)) AS scheduled_meetings_organized,
    toInt64(COALESCE(scheduledOneTimeMeetingsAttendedCount, 0)
        + COALESCE(scheduledRecurringMeetingsAttendedCount, 0)) AS scheduled_meetings_attended,
    toInt64({{ iso8601_duration_seconds("ifNull(audioDuration, 'PT0S')") }}) AS audio_duration_seconds,
    toInt64({{ iso8601_duration_seconds("ifNull(videoDuration, 'PT0S')") }}) AS video_duration_seconds,
    toInt64({{ iso8601_duration_seconds("ifNull(screenShareDuration, 'PT0S')") }}) AS screen_share_duration_seconds,
    reportPeriod AS report_period,
    now() AS collected_at,
    'insight_m365' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM {{ source('bronze_m365', 'teams_activity') }}
WHERE userPrincipalName IS NOT NULL
  AND userPrincipalName != ''
{% if is_incremental() %}
  AND (
    (SELECT max(date) FROM {{ this }}) IS NULL
    OR toDate(reportRefreshDate) > (SELECT max(date) - INTERVAL 3 DAY FROM {{ this }})
  )
{% endif %}
