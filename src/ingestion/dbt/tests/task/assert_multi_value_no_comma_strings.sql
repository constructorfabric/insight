-- A multi-value element must never be an unsplit list.
--
-- The failure this hunts is real: Jira serializes some multi-value fields as one
-- item carrying the whole list, and pushing that item through as a single
-- element yields `value_ids = ['8066, 8363']` — one element that is actually
-- two. The arrays stay parallel and carry no duplicate, so neither
-- `assert_value_arrays_same_length` nor `assert_no_duplicate_items_in_array`
-- notices; this test is the only one that does.
--
-- The check is on the ID side, not the display side. That distinction is the
-- point:
--
--   * an id is opaque or numeric — an option id, a version id, an account id, a
--     label. `", "` inside one can only mean a list was not split. A label,
--     which is its own id, cannot contain a space at all, so it cannot contain
--     `", "` either.
--   * a DISPLAY legitimately contains `", "`. Component and customer names do
--     it routinely, and so do sprint names. Checking displays reported every
--     such name as a defect — it passed before only because the fields carrying
--     those names were missing from the modelled history altogether.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    field_name,
    event_id,
    value_ids,
    value_displays
FROM silver.class_task_field_history FINAL
WHERE field_cardinality = 'multi'
  AND arrayExists(v -> position(v, ', ') > 0, value_ids)
LIMIT 100
