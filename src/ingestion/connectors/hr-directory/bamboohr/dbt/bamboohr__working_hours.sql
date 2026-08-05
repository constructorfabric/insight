-- depends_on: {{ ref('bamboohr__bronze_promoted') }}
{{ config(
    materialized='view',
    schema='staging',
    tags=['bamboohr', 'silver:class_hr_working_hours']
) }}

SELECT
    tenant_id                 AS insight_tenant_id,
    source_id,
    unique_key,
    id                        AS source_person_id,
    workEmail                 AS email,
    COALESCE(displayName, workEmail) AS display_name,
    employmentHistoryStatus   AS employment_type,
    'bamboohr'                AS source,
    -- standardHoursPerWeek is not provided by this tenant; defaulting to 8h/day full-time
    toFloat64(8.0)            AS working_hours_per_day,
    toFloat64(40.0)           AS working_hours_per_week,
    _airbyte_extracted_at     AS ingested_at
-- FINAL is mandatory: bronze is append-only RMT(_airbyte_extracted_at) and only
-- collapses on background merge. Without it the `status = 'Active'` filter below
-- is evaluated against EVERY unmerged snapshot, so an employee who has since gone
-- inactive still qualifies via a stale row — and the downstream versionless
-- `LIMIT 1 BY unique_key` has no ORDER BY, so which snapshot's hours win is
-- undefined. See ADR-0001.
FROM {{ source('bamboohr', 'employees') }} FINAL
WHERE status = 'Active'
  AND id IS NOT NULL
  AND workEmail IS NOT NULL
