{{ config(materialized='table', engine='ReplacingMergeTree(_version)', order_by=['unique_key'], settings={'allow_nullable_key': 1}, schema='staging', tags=['youtrack', 'staging', 'silver:class_task_worklogs']) }}

SELECT w.unique_key AS unique_key, w.source_id AS insight_source_id, 'youtrack' AS data_source,
    w.id AS worklog_id, i.id_readable AS id_readable, w.author_id AS author_id,
    fromUnixTimestamp64Milli(toInt64(w.date)) AS work_date,
    CAST(toFloat64OrNull(JSONExtractRaw({{ youtrack_json('w.work_item_json') }}, 'duration', 'minutes')) * 60 AS Nullable(Float64)) AS duration_seconds,
    nullIf(JSONExtractString({{ youtrack_json('w.work_item_json') }}, 'text'), '') AS description,
    toDateTime64(w._airbyte_extracted_at, 3) AS collected_at,
    CAST(NULL AS Nullable(UInt8)) AS is_deleted,
    toUnixTimestamp64Milli(now64(3)) AS _version,
    CAST(w.issue_id AS Nullable(String)) AS issue_id
FROM {{ source('bronze_youtrack', 'youtrack_work_items') }} AS w FINAL
LEFT JOIN {{ ref('youtrack__issues') }} AS i ON i.insight_source_id = w.source_id AND i.issue_id = w.issue_id
