-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='github__task_statuses',
    schema='staging',
    tags=['github', 'staging', 'silver:class_task_statuses']
) }}

-- Per-source status dimension; unioned into `silver.class_task_statuses` via
-- `union_by_tag`. GitHub states no lifecycle category anywhere, so which
-- category a value means is an operator's decision, not the vendor's — every
-- row here comes from the binding.
--
-- Two independent status families, deliberately not merged. The issue's own
-- lifecycle is open or closed and a closure states a reason: that pair is the
-- status. A Projects V2 board additionally has its own status column, one
-- field per board, which is where an `in_progress` category can come from. An
-- issue can carry both; they are separate fields with separate bindings, and
-- which one wins for a measure is the `precedence` column's job, not this
-- model's.
--
-- INVARIANT: a value observed in history with no binding must NOT default to a
-- category. It is left out, the coverage test names it, and the build fails —
-- the alternative is an issue that silently never closes.

SELECT
    CAST(concat(m.tenant_id, ':', m.insight_source_id, ':github:status:', m.value_id) AS Nullable(String)) AS unique_key,
    CAST(m.insight_source_id AS Nullable(String))           AS insight_source_id,
    CAST('github' AS String)                                AS data_source,
    CAST(m.value_id AS Nullable(String))                    AS status_id,
    CAST(nullIf(m.value_display, '') AS Nullable(String))   AS status_name,
    CAST(NULL AS Nullable(Int32))                           AS category_id,
    CAST(NULL AS Nullable(String))                          AS category_key,
    CAST(m.canonical_value AS String)                       AS status_category,
    now64(3)                                                AS collected_at,
    toUnixTimestamp64Milli(now64(3))                        AS _version
FROM (
    SELECT
        tenant_id,
        insight_source_id,
        value_id,
        value_display,
        canonical_value
    FROM {{ source('config', 'task_value_map') }} FINAL
    WHERE data_source = 'github'
      -- Two field families reach this dimension, and they stay separate: the
      -- issue's own open/closed `state`, and one status field per Projects V2
      -- board. A board's value key already carries the board, so both families
      -- key on `value_id` alone and cannot collide.
      AND (field_id = 'state' OR startsWith(field_id, 'project_status:'))
      AND is_deleted = 0
      AND valid_from <= now64(3)
    ORDER BY valid_from DESC, recorded_at DESC
    LIMIT 1 BY tenant_id, insight_source_id, value_id
) AS m
