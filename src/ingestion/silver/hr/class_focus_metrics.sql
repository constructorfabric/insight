{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

SELECT
    ma.tenant_id                                                    AS insight_tenant_id,
    ma.person_key                                                   AS email,
    ma.date                                                         AS day,
    concat(
        ma.tenant_id, '-',
        ma.person_key, '-',
        toString(ma.date)
    )                                                               AS unique_key,
    toInt64(sum(ma.meetings_attended + ifNull(ma.calls_count, 0)))  AS meetings_count,
    -- Use the longest modality (audio / video / screen-share) to avoid under-
    -- counting M365 Teams participants who joined muted but with camera or
    -- screen-share on. For Zoom, audio_duration is the time with company present
    -- and always dominates, so greatest(...) reduces to audio.
    ROUND(
        sum(greatest(
            ma.audio_duration_seconds,
            ma.video_duration_seconds,
            ma.screen_share_duration_seconds
        )) / 3600.0,
        4
    )                                                               AS meeting_hours,
    -- WORKAROUND: under join_use_nulls=0 an unmatched LEFT JOIN yields 0, not NULL.
    COALESCE(nullIf(wh.working_hours_per_day, 0), 8.0)              AS working_hours_per_day,
    ROUND(
        GREATEST(toFloat64(0), 100.0 - (
            sum(greatest(
                ma.audio_duration_seconds,
                ma.video_duration_seconds,
                ma.screen_share_duration_seconds
            ))
            / 3600.0
            / nullIf(COALESCE(nullIf(wh.working_hours_per_day, 0), 8.0), 0)
        ) * 100.0),
        2
    )                                                               AS focus_time_pct,
    ROUND(
        GREATEST(toFloat64(0),
            COALESCE(nullIf(wh.working_hours_per_day, 0), 8.0) -
            sum(greatest(
                ma.audio_duration_seconds,
                ma.video_duration_seconds,
                ma.screen_share_duration_seconds
            )) / 3600.0
        ),
        4
    )                                                               AS dev_time_h,
    toUnixTimestamp64Milli(now64())                                 AS _version,
    max(ma._version)                                                AS source_version
FROM {{ ref('class_collab_meeting_activity') }} ma FINAL
LEFT JOIN {{ ref('class_hr_working_hours') }} wh FINAL
    ON ma.person_key = lower(wh.email)
   AND ma.tenant_id = wh.insight_tenant_id
WHERE ma.person_key != ''
  AND ma.date IS NOT NULL
{% if is_incremental() %}
  -- INVARIANT: the recent days pick up working-hours changes; any older person-day is
  -- re-derived once a meeting row newer than the one it was derived from arrives.
  AND (
      ma.date > (SELECT max(day) - INTERVAL 3 DAY FROM {{ this }})
      OR (ma.tenant_id, ma.person_key, ma.date) IN (
          SELECT
              changed.tenant_id,
              changed.person_key,
              changed.date
          FROM {{ ref('class_collab_meeting_activity') }} AS changed FINAL
          LEFT JOIN {{ this }} AS derived FINAL
              ON derived.insight_tenant_id = changed.tenant_id
              AND derived.email = changed.person_key
              AND derived.day = changed.date
          WHERE changed._version > ifNull(derived.source_version, 0)
      )
  )
{% endif %}
GROUP BY
    ma.tenant_id,
    ma.person_key,
    ma.date,
    COALESCE(nullIf(wh.working_hours_per_day, 0), 8.0)
