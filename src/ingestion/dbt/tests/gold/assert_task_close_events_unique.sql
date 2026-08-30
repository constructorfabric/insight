-- One row per close transition: an issue closed more than once contributes one
-- row per close time, never two rows for the same one.
SELECT
    insight_source_id,
    issue_id,
    close_at,
    count() AS row_count
FROM {{ ref('task_close_events') }}
GROUP BY insight_source_id, issue_id, close_at
HAVING count() > 1
