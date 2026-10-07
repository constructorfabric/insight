{{ config(materialized='table', engine=insight_engine('ReplacingMergeTree', '_version'), order_by=['unique_key'], settings={'allow_nullable_key': 1}, schema='staging', tags=['youtrack', 'staging', 'silver:class_task_sprints']) }}

SELECT unique_key, source_id AS insight_source_id, 'youtrack' AS data_source,
    id AS sprint_id, agile_id AS board_id,
    nullIf(JSONExtractString({{ youtrack_json('sprint_json') }}, 'agile', 'name'), '') AS board_name,
    name AS sprint_name, CAST(NULL AS Nullable(String)) AS project_key,
    CAST(NULL AS Nullable(String)) AS state,
    fromUnixTimestamp64Milli(toInt64(start)) AS start_date,
    fromUnixTimestamp64Milli(toInt64(finish)) AS end_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS complete_date,
    toDateTime64(_airbyte_extracted_at, 3) AS collected_at, toUnixTimestamp64Milli(now64(3)) AS _version FROM {{ source('bronze_youtrack', 'youtrack_sprints') }} FINAL
