{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

WITH snapshots AS (
    SELECT insight_source_id, issue_id, id_readable, observed_at,
        arrayFilter(x -> x != '', arrayMap(f -> JSONExtractString(f, 'projectCustomField', 'field', 'id'),
            JSONExtractArrayRaw(custom_fields))) AS field_ids
    FROM {{ ref('youtrack__issue_observations') }} FINAL
    WHERE JSONType(custom_fields) = 'Array'
), previous AS (
    SELECT *, lagInFrame(field_ids) OVER (PARTITION BY insight_source_id, issue_id ORDER BY observed_at
        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS previous_ids
    FROM snapshots
)
SELECT insight_source_id, issue_id, id_readable, observed_at,
    arrayJoin(arrayFilter(f -> NOT has(field_ids, f), previous_ids)) AS field_id
FROM previous
