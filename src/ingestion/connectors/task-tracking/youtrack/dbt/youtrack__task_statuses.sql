{{ config(materialized='table', engine='ReplacingMergeTree(_version)', order_by=['unique_key'], settings={'allow_nullable_key': 1}, schema='staging', tags=['youtrack', 'staging', 'silver:class_task_statuses']) }}

SELECT CAST(concat(insight_source_id, '-youtrack-status-', value_id) AS Nullable(String)) AS unique_key,
    CAST(insight_source_id AS Nullable(String)) AS insight_source_id, 'youtrack' AS data_source,
    CAST(value_id AS Nullable(String)) AS status_id, CAST(value_display AS Nullable(String)) AS status_name,
    CAST(NULL AS Nullable(Int32)) AS category_id, CAST(NULL AS Nullable(String)) AS category_key,
    CAST(canonical_value AS String) AS status_category, now64(3) AS collected_at,
    toUnixTimestamp64Milli(now64(3)) AS _version
FROM (
    SELECT * FROM {{ source('config', 'task_value_map') }} FINAL
    WHERE data_source = 'youtrack' AND valid_from <= now64(3)
    ORDER BY valid_from DESC, recorded_at DESC
    LIMIT 1 BY insight_source_id, field_id, value_id
)
WHERE is_deleted = 0 AND canonical_value IN ('new', 'in_progress', 'done', 'undefined')
