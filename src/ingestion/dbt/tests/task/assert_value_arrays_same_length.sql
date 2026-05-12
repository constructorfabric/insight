-- Every row must have parallel arrays — same number of ids and displays.
-- Failures indicate a bug in `core::emit_*_row` or snapshot accumulator.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_id,
    length(value_ids)      AS n_ids,
    length(value_displays) AS n_displays
FROM silver.class_task_field_history FINAL
WHERE length(value_ids) != length(value_displays)
LIMIT 100
