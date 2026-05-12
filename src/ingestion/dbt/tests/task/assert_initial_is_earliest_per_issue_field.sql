-- Initial rows must be the earliest event per (issue, field). If any changelog row has
-- event_at earlier than the corresponding initial row, something is wrong with the
-- reconstruct step or the snapshot's created_at was miscomputed.

WITH per_key AS (
    SELECT
        insight_source_id,
        data_source,
        issue_id,
        field_id,
        min(CASE WHEN event_kind = 'synthetic_initial'   THEN event_at END) AS first_initial,
        min(CASE WHEN event_kind = 'changelog' THEN event_at END) AS first_changelog
    FROM silver.class_task_field_history FINAL
    GROUP BY insight_source_id, data_source, issue_id, field_id
)
-- Small clock-skew tolerance: Jira occasionally returns `changelog.created` a few
-- seconds before `issue.created` (server clock drift, data import). Real bugs show
-- much larger gaps — here we only flag skew > 60 seconds.
SELECT *
FROM per_key
WHERE first_initial IS NOT NULL
  AND first_changelog IS NOT NULL
  AND first_initial > addSeconds(first_changelog, 60)
LIMIT 100
