{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='append',
    engine='ReplacingMergeTree(_version)',
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
    toInt64(sum(ma.meetings_attended))                              AS meetings_count,
    -- Use the longest modality (audio / video / screen-share) to avoid under-
    -- counting M365 Teams participants who joined muted but with camera or
    -- screen-share on. For Zoom, audio_duration is full participation time
    -- and always dominates, so greatest(...) reduces to audio.
    ROUND(
        sum(greatest(
            ma.audio_duration_seconds,
            ma.video_duration_seconds,
            ma.screen_share_duration_seconds
        )) / 3600.0,
        4
    )                                                               AS meeting_hours,
    COALESCE(wh.working_hours_per_day, 8.0)                        AS working_hours_per_day,
    ROUND(
        GREATEST(toFloat64(0), 100.0 - (
            sum(greatest(
                ma.audio_duration_seconds,
                ma.video_duration_seconds,
                ma.screen_share_duration_seconds
            ))
            / 3600.0
            / nullIf(COALESCE(wh.working_hours_per_day, 8.0), 0)
        ) * 100.0),
        2
    )                                                               AS focus_time_pct,
    ROUND(
        GREATEST(toFloat64(0),
            COALESCE(wh.working_hours_per_day, 8.0) -
            sum(greatest(
                ma.audio_duration_seconds,
                ma.video_duration_seconds,
                ma.screen_share_duration_seconds
            )) / 3600.0
        ),
        4
    )                                                               AS dev_time_h,
    toUnixTimestamp64Milli(now64())                                 AS _version
FROM {{ ref('class_collab_meeting_activity') }} ma
LEFT JOIN {{ ref('class_hr_working_hours') }} wh
    ON ma.person_key = lower(wh.email)
   AND ma.tenant_id = wh.insight_tenant_id
WHERE ma.person_key != ''
  AND ma.date IS NOT NULL
{% if is_incremental() %}
  AND ma.date
      > (SELECT max(day) - INTERVAL 3 DAY FROM {{ this }})
{% endif %}
GROUP BY
    ma.tenant_id,
    ma.person_key,
    ma.date,
    COALESCE(wh.working_hours_per_day, 8.0)
