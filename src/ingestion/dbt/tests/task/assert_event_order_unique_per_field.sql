-- `event_order` is the only key a reader sorts one field's history by, so two
-- rows of one (issue, field) sharing it leave that field's order undefined.
-- Rows of different fields may share it: every field one changelog entry
-- changed carries the entry's position.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_order,
    count() AS n
FROM silver.class_task_field_history FINAL
GROUP BY insight_source_id, data_source, issue_id, field_id, event_order
HAVING n > 1
LIMIT 100
