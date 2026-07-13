-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='youtrack__task_worklogs',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['youtrack', 'silver:class_task_worklogs']
) }}

-- Per-source staging projection; unioned into `silver.class_task_worklogs`.
-- Worklogs come from an append/incremental substream → incremental RMT table
-- (mirrors jira__task_worklogs). Bronze deduped by `_airbyte_raw_id` before append.
-- `id_readable` resolved via a deduped join to `youtrack_issue` (bronze worklogs
-- carry only the internal issue id). YouTrack logs effort in minutes →
-- duration_seconds = minutes * 60. `date` is epoch-ms.

SELECT
    w.unique_key                                            AS unique_key,
    w.source_id                                             AS insight_source_id,
    CAST('youtrack' AS String)                              AS data_source,
    toString(w.worklog_id)                                  AS worklog_id,
    COALESCE(i.id_readable, toString(w.youtrack_id))        AS id_readable,
    w.author_id                                             AS author_id,
    fromUnixTimestamp64Milli(toInt64OrNull(toString(w.date)))    AS work_date,
    toFloat64OrNull(toString(w.duration_minutes)) * 60      AS duration_seconds,
    w.text                                                  AS description,
    toDateTime64(w._airbyte_extracted_at, 3)                AS collected_at,
    toUnixTimestamp64Milli(now64(3))                        AS _version
FROM (
    SELECT * FROM {{ source('bronze_youtrack', 'youtrack_worklogs') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY _airbyte_raw_id
) w
LEFT JOIN (
    SELECT source_id, youtrack_id, id_readable
    FROM {{ source('bronze_youtrack', 'youtrack_issue') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY source_id, youtrack_id
) i ON i.source_id = w.source_id AND i.youtrack_id = w.youtrack_id
