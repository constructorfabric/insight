{{ config(materialized='table', engine='ReplacingMergeTree(_version)', order_by=['unique_key'], settings={'allow_nullable_key': 1}, schema='staging', tags=['youtrack', 'staging', 'silver:class_task_comments']) }}

SELECT unique_key, source_id AS insight_source_id, 'youtrack' AS data_source,
    id AS comment_id, issue_id_readable AS id_readable, author_id,
    fromUnixTimestamp64Milli(toInt64(created)) AS created_at,
    {{ youtrack_timestamp('updated') }} AS updated_at, text AS body,
    CAST(deleted AS Nullable(UInt8)) AS is_deleted,
    toUnixTimestamp64Milli(now64(3)) AS _version,
    CAST(issue_id AS Nullable(String)) AS issue_id
FROM {{ source('bronze_youtrack', 'youtrack_comments') }} FINAL
