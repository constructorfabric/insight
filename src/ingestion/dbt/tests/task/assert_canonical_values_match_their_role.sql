-- A canonical value must fall inside the domain its field's role permits.
-- `role` is a low-cardinality string rather than an enum, so the database
-- constrains neither it nor the values bound under it — this test is what
-- carries that constraint instead of the schema.

WITH bound AS (
    SELECT
        insight_source_id,
        data_source,
        field_id,
        argMax(role, (valid_from, recorded_at)) AS role
    FROM config.task_field_roles FINAL
    WHERE is_deleted = 0 AND valid_from <= now64(3)
    GROUP BY insight_source_id, data_source, field_id
)

SELECT
    m.insight_source_id,
    m.data_source,
    m.field_id,
    m.value_id,
    b.role,
    m.canonical_value
FROM config.task_value_map AS m FINAL
INNER JOIN bound AS b
    ON b.insight_source_id = m.insight_source_id
    AND b.data_source = m.data_source
    AND b.field_id = m.field_id
WHERE m.is_deleted = 0
  AND NOT (
      (b.role = 'status'    AND m.canonical_value IN ('new', 'in_progress', 'done', 'undefined'))
      OR (b.role = 'issuetype' AND m.canonical_value IN ('bug', 'other', 'unknown'))
      OR b.role NOT IN ('status', 'issuetype')
  )
LIMIT 100
