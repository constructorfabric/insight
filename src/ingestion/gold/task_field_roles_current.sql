{{ config(
    materialized='ephemeral',
    tags=['gold']
) }}

-- Which vendor field plays which metric role, resolved to one row per
-- (source, field). Gold matches a role, never a vendor's field identifier —
-- `field_id` is documented as vendor-specific, so a literal like `'status'` is
-- Jira's name for the thing and no other source can honour it.
--
-- Ephemeral because it is small enough to inline wherever it is joined and
-- there is nothing to gain from materializing a handful of rows.
--
-- Resolution is as of the build, not as of the event: the row in force now
-- applies to all history. That is the correction semantics — an operator fixing
-- a wrong binding expects the fix to reach the numbers already published. The
-- validity axis is recorded and becomes effective when the class dimensions
-- gain a period grain; see the design's configuration section.
--
-- `tenant_id` is deliberately absent from the output. `class_task_field_history`
-- carries no tenant column — a source instance belongs to exactly one tenant,
-- so the source identifier already answers for it.

SELECT
    insight_source_id,
    data_source,
    field_id,
    argMax(role, (valid_from, recorded_at))            AS role,
    argMax(precedence, (valid_from, recorded_at))      AS precedence,
    argMax(value_unit, (valid_from, recorded_at))      AS value_unit,
    argMax(unit_multiplier, (valid_from, recorded_at)) AS unit_multiplier
FROM {{ source('config', 'task_field_roles') }} FINAL
WHERE is_deleted = 0
  AND valid_from <= now64(3)
GROUP BY insight_source_id, data_source, field_id
HAVING role != 'ignored'
