{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'One issue type row per source and type id',
        'domain': 'task-tracking',
        'category': 'grain',
        'tier': 'warn',
        'remediation': 'class_task_issuetypes holds more than one row for a source and issue type id, so every gold join on issue type multiplies issues. Check the producer\'s unique_key formula for that source.'
    }
) }}

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
