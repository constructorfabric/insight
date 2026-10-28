{{ config(
    materialized='table',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['staging', 'jira']
) }}

WITH issues AS (
    SELECT
        CAST(source_id AS String)                                     AS insight_source_id,
        CAST(jira_id AS String)                                       AS issue_id,
        argMax(COALESCE(toString(id_readable), ''), _airbyte_extracted_at) AS current_key,
        argMax(parseDateTime64BestEffortOrNull(created, 3), _airbyte_extracted_at) AS created_at,
        argMax(parseDateTime64BestEffortOrNull(updated, 3), _airbyte_extracted_at) AS updated_at,
        toDateTime64(max(_airbyte_extracted_at), 3)                   AS read_at
    FROM {{ source('bronze_jira', 'jira_issue') }}
    GROUP BY insight_source_id, issue_id
)

SELECT
    concat(insight_source_id, '-', issue_id) AS unique_key,
    insight_source_id,
    issue_id,
    current_key,
    created_at,
    updated_at,
    read_at,
    toUnixTimestamp64Milli(now64(3)) AS _version
FROM issues
