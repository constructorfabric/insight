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
FROM {{ source('bamboohr', 'employees') }}
WHERE status = 'Active'
  AND id IS NOT NULL
  AND workEmail IS NOT NULL
