{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Mapped status values are canonical states',
        'domain': 'task-tracking',
        'category': 'configuration',
        'tier': 'error',
        'remediation': 'A value map entry for a status-role field targets a value outside new, in_progress, done and undefined, so gold cannot classify issues in it. Correct the target_value of the entry named in the finding.'
    }
) }}

-- A canonical value must fall inside the domain its field's role permits.
-- `role` is a low-cardinality string rather than an enum, so the database
-- constrains neither it nor the values bound under it — this test is what
-- carries that constraint instead of the schema.
--
-- Only statuses reach `task_value_map`; issue-type decisions live in
-- `config.field_value_map`, whose domain
-- `assert_field_values_are_canonical` carries.

WITH bound AS (
    SELECT
        insight_source_id,
        data_source,
        field_id,
        argMax(role, (valid_from, recorded_at)) AS role
    FROM config.task_field_roles FINAL
    WHERE is_deleted = 0 AND valid_from <= now64(3)
    GROUP BY insight_source_id, data_source, field_id
),

latest_mappings AS (
    SELECT
        insight_source_id,
        data_source,
        field_id,
        value_id,
        canonical_value,
        is_deleted
    FROM config.task_value_map FINAL
    ORDER BY recorded_at DESC
    LIMIT 1 BY tenant_id, insight_source_id, data_source, field_id, value_id, valid_from
)

SELECT
    m.insight_source_id,
    m.data_source,
    m.field_id,
    m.value_id,
    b.role,
    m.canonical_value
FROM latest_mappings AS m
INNER JOIN bound AS b
    ON b.insight_source_id = m.insight_source_id
    AND b.data_source = m.data_source
    AND b.field_id = m.field_id
WHERE m.is_deleted = 0
  AND NOT (
      (b.role = 'status' AND m.canonical_value IN ('new', 'in_progress', 'done', 'undefined'))
      OR b.role != 'status'
  )
