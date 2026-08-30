-- An issue id is unique only within the source that tracks it, and an open
-- issue contributes exactly one row.
SELECT
    insight_source_id,
    issue_id,
    count() AS row_count
FROM {{ ref('task_open_issues') }}
GROUP BY insight_source_id, issue_id
HAVING count() > 1
