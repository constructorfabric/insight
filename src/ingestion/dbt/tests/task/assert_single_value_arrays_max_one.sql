-- Single-value field rows must have at most one element in value_ids / value_displays.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_id,
    length(value_ids) AS n
FROM silver.class_task_field_history FINAL
WHERE field_cardinality = 'single'
  AND length(value_ids) > 1
LIMIT 100
