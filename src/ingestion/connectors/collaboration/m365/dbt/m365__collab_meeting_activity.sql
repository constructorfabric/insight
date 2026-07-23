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
  -- Drop unlicensed users (guests, ex-employees, service accounts). Their Teams
  -- activity inflates team-level collab counters and produces orphan gold rows
  -- with no matching insight.people entry. `isLicensed` = "Selected if the user
  -- is licensed to use Teams" (MS Graph getTeamsUserActivityUserDetail). Only the
  -- teams_activity report exposes this flag; the email/onedrive/sharepoint feeders
  -- fall back to `assignedProducts`. A NULL flag is treated as unlicensed and
  -- dropped too, per the #736 proposal. See #736.
  AND coalesce(isLicensed, false) = true
{% if is_incremental() %}
  -- Watermark on the source EXTRACT time, not the business date: a forward-only
  -- `reportRefreshDate > max(date) - 3d` filter permanently strands backfilled /
  -- late-arriving history (older business dates never re-enter the model — see the
  -- zoom model header). Re-pulled rows carry a fresh `_airbyte_extracted_at`, so we
  -- reprocess every business date touched by a recent extract; cheap in steady state.
  AND (
    (SELECT count() FROM {{ this }}) = 0
    OR toDate(reportRefreshDate) IN (
      SELECT DISTINCT toDate(reportRefreshDate)
      FROM {{ source('bronze_m365', 'teams_activity') }}
      WHERE _airbyte_extracted_at
            > (SELECT max(_airbyte_extracted_at) FROM {{ source('bronze_m365', 'teams_activity') }}) - INTERVAL 3 DAY
    )
  )
{% endif %}
