-- A title's value and its display are the same string.

-- The title has no identifier apart from the text: `value_id_type` says
-- `string_literal` and the literal IS the title. So the two arrays must agree
-- on every row bound to the `title` role, whatever the source.
--
-- The failure this hunts is silent and one-sided. A source that reconstructs
-- the value at creation — rewinding it to the previous title of the first
-- rename — while carrying the display from the issue's CURRENT snapshot
-- reports the two channels differently on that one row. Gold reads
-- `value_displays[1]`, so the issue's creation row claims the name it has
-- today, and every array-shape test still passes because the arrays stay
-- parallel and hold no duplicate.

SELECT
    fh.insight_source_id,
    fh.data_source,
    fh.issue_id,
    fh.field_id,
    fh.event_kind,
    fh.event_id,
    fh.value_ids,
    fh.value_displays
FROM silver.class_task_field_history AS fh FINAL
INNER JOIN {{ ref('task_field_roles_current') }} AS r
    ON r.insight_source_id = fh.insight_source_id
   AND r.data_source = fh.data_source
   AND r.field_id = fh.field_id
WHERE r.role = 'title'
  AND length(fh.value_ids) = 1
  AND length(fh.value_displays) = 1
  AND fh.value_displays[1] != ''
  AND fh.value_ids[1] != fh.value_displays[1]
LIMIT 100
