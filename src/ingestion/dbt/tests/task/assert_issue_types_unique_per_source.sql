-- One row per (insight_source_id, issue_type_id): the dimension gold joins by
-- that pair, so a duplicate fans out every issue of the type. FINAL collapses
-- same-`unique_key` ReplacingMergeTree parts first; a row that survives the
-- grouping means two distinct unique_key values describe the same type —
-- a wrong key formula in a per-source projection, not a merge artifact.

SELECT
    insight_source_id,
    issue_type_id,
    count() AS n
FROM {{ ref('class_task_issuetypes') }} FINAL
GROUP BY insight_source_id, issue_type_id
HAVING n > 1
LIMIT 100
