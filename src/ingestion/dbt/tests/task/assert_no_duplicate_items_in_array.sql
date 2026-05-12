-- value_ids must not contain duplicates within the same row.
-- A duplicate indicates either a buggy Add for a value already present, or a misapplied
-- Snapshot that preserved duplicates.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_id,
    value_ids,
    length(value_ids)              AS n,
    length(arrayDistinct(value_ids)) AS n_distinct
FROM silver.class_task_field_history FINAL
WHERE length(value_ids) != length(arrayDistinct(value_ids))
LIMIT 100
