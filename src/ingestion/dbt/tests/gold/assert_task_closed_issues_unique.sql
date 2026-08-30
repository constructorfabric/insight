-- An issue id is unique only within the source that tracks it, and a closed
-- issue contributes exactly one row.
SELECT
    insight_source_id,
    issue_id,
    count() AS row_count
FROM {{ ref('task_closed_issues') }}
GROUP BY insight_source_id, issue_id
HAVING count() > 1
