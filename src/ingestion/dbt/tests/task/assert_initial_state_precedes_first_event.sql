-- The state at creation is by definition the state before every event, even
-- when an imported history dates an event before the issue's own creation.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    maxIf(event_order, event_kind = 'synthetic_initial') AS last_initial_order,
    minIf(event_order, event_kind = 'changelog')         AS first_event_order
FROM silver.class_task_field_history FINAL
GROUP BY insight_source_id, data_source, issue_id
HAVING countIf(event_kind = 'synthetic_initial') > 0
   AND countIf(event_kind = 'changelog') > 0
   AND last_initial_order >= first_event_order
LIMIT 100
