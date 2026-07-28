SELECT
    insight_source_id,
    issue_id,
    count() AS row_count
FROM {{ ref('task_issue_state') }}
GROUP BY insight_source_id, issue_id
HAVING count() > 1
