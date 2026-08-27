-- A field carrying events, in a source that has declared any binding at all,
-- must be bound to a role or explicitly marked `ignored`. The distinction is
-- the point: `ignored` says somebody looked and decided, silence says nobody
-- has. Only silence is a finding.
--
-- The `created` sentinel is not a field and never appears in the catalogue.

WITH configured_sources AS (
    SELECT DISTINCT insight_source_id, data_source
    FROM config.task_field_roles FINAL
    WHERE is_deleted = 0
),

declared AS (
    SELECT DISTINCT insight_source_id, data_source, field_id
    FROM config.task_field_roles FINAL
    WHERE is_deleted = 0 AND valid_from <= now64(3)
),

carrying_events AS (
    SELECT DISTINCT
        fh.insight_source_id AS insight_source_id,
        fh.data_source       AS data_source,
        fh.field_id          AS field_id
    FROM silver.class_task_field_history AS fh FINAL
    INNER JOIN configured_sources AS c
        ON c.insight_source_id = fh.insight_source_id
        AND c.data_source = fh.data_source
    WHERE fh.field_id != 'created'
      {% if not var('task_board_bindings_enforced', false) %}
      -- Boards are not enforced yet: one authored binding per board, and no
      -- surface to author them in. See `task_board_bindings_enforced`.
      AND NOT startsWith(fh.field_id, 'project_status:')
      {% endif %}
)

SELECT e.*
FROM carrying_events AS e
LEFT ANTI JOIN declared AS d
    ON d.insight_source_id = e.insight_source_id
    AND d.data_source = e.data_source
    AND d.field_id = e.field_id
LIMIT 100
