{{ config(materialized='table', engine=insight_engine('ReplacingMergeTree', '_version'), order_by=['unique_key'], settings={'allow_nullable_key': 1}, schema='staging', tags=['youtrack', 'staging', 'silver:class_task_comments']) }}

WITH comments AS (
    -- WORKAROUND: rows written before the stream carried `id`/`issue_id` keep only
    -- their key, `{tenant}-{source}-{comment id}`; the issue comes from the snapshots.
    SELECT *, coalesce(id, substring(unique_key, length(concat(tenant_id, '-', source_id, '-')) + 1)) AS resolved_comment_id
    FROM {{ source('bronze_youtrack', 'youtrack_comments') }} FINAL
)
SELECT c.unique_key AS unique_key, c.source_id AS insight_source_id, 'youtrack' AS data_source,
    c.resolved_comment_id AS comment_id, coalesce(c.issue_id_readable, nullIf(i.id_readable, '')) AS id_readable, c.author_id AS author_id,
    fromUnixTimestamp64Milli(toInt64(c.created)) AS created_at,
    {{ youtrack_timestamp('c.updated') }} AS updated_at, c.text AS body,
    CAST(c.deleted AS Nullable(UInt8)) AS is_deleted,
    toUnixTimestamp64Milli(now64(3)) AS _version,
    CAST(coalesce(c.issue_id, nullIf(m.issue_id, '')) AS Nullable(String)) AS issue_id
FROM comments AS c
LEFT JOIN {{ ref('youtrack__comment_issues') }} AS m
    ON m.insight_source_id = c.source_id AND m.comment_id = c.resolved_comment_id
LEFT JOIN {{ ref('youtrack__issues') }} AS i
    ON i.insight_source_id = c.source_id AND i.issue_id = coalesce(c.issue_id, nullIf(m.issue_id, ''))
