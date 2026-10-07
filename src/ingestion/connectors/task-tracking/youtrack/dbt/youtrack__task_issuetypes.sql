{{ config(materialized='table', engine=insight_engine('ReplacingMergeTree', '_version'), order_by=['unique_key'], settings={'allow_nullable_key': 1}, schema='staging', tags=['youtrack', 'staging', 'silver:class_task_issuetypes']) }}

SELECT CAST(concat(v.insight_source_id, '-youtrack-type-', v.value_id) AS Nullable(String)) AS unique_key,
    CAST(v.insight_source_id AS Nullable(String)) AS insight_source_id, 'youtrack' AS data_source,
    CAST(v.value_id AS Nullable(String)) AS issue_type_id,
    CAST(v.value_name AS Nullable(String)) AS issue_type_name,
    CAST(NULL AS Nullable(String)) AS untranslated_name,
    v.observed_at AS collected_at, toUnixTimestamp64Milli(now64(3)) AS _version
FROM {{ ref('youtrack__bundle_values') }} AS v
INNER JOIN (
    SELECT * FROM (
        SELECT * FROM {{ source('config', 'task_field_roles') }} FINAL
        WHERE data_source = 'youtrack' AND valid_from <= now64(3)
        ORDER BY valid_from DESC, recorded_at DESC
        LIMIT 1 BY insight_source_id, field_id
    ) WHERE is_deleted = 0 AND role = 'issuetype'
) AS r ON r.insight_source_id = v.insight_source_id AND r.field_id = v.field_id
