-- Multi-value field elements should never contain ", " inside a single item.
-- A failing row = Jira emitted a legacy comma-list in fromString/toString and the enricher
-- pushed the entire comma string as one element (Sprint/Roadmap bug pattern).
-- After the fix, such rows should be 0 for existing sources.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    field_name,
    event_id,
    value_displays
FROM silver.class_task_field_history FINAL
WHERE field_cardinality = 'multi'
  AND arrayExists(v -> position(v, ', ') > 0, value_displays)
LIMIT 100
