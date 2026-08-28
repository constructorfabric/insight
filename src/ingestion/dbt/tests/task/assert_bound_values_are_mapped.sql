-- Every value of a field bound to a lifecycle role must have a mapping. An
-- unmapped status value is worse than an error: it produces no dimension row,
-- gold treats anything that is not `done` as open, and the issue never closes —
-- a wrong number that looks like a quiet one.
--
-- Only sources that have declared any binding are checked. A source with no
-- configuration at all derives its categories from the vendor and owes nothing
-- here.

WITH bound AS (
    SELECT
        insight_source_id,
        data_source,
        field_id,
        argMax(role, (valid_from, recorded_at)) AS role
    FROM config.task_field_roles FINAL
    WHERE is_deleted = 0 AND valid_from <= now64(3)
    GROUP BY insight_source_id, data_source, field_id
    HAVING role IN ('status', 'issuetype')
),

mapped AS (
    SELECT DISTINCT insight_source_id, data_source, field_id, value_id
    FROM config.task_value_map FINAL
    WHERE is_deleted = 0 AND valid_from <= now64(3)
),

observed AS (
    SELECT DISTINCT
        fh.insight_source_id AS insight_source_id,
        fh.data_source       AS data_source,
        fh.field_id          AS field_id,
        fh.value_ids[1]      AS value_id
    FROM silver.class_task_field_history AS fh FINAL
    INNER JOIN bound AS b
        ON b.insight_source_id = fh.insight_source_id
        AND b.data_source = fh.data_source
        AND b.field_id = fh.field_id
    WHERE fh.delta_action = 'set' AND fh.value_ids[1] != ''
      {% if not var('task_board_bindings_enforced', false) %}
      AND NOT startsWith(fh.field_id, 'project_status:')
      {% endif %}
)

SELECT o.*
FROM observed AS o
LEFT ANTI JOIN mapped AS m
    ON m.insight_source_id = o.insight_source_id
    AND m.data_source = o.data_source
    AND m.field_id = o.field_id
    AND m.value_id = o.value_id
LIMIT 100
