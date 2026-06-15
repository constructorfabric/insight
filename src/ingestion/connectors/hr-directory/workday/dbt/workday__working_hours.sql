-- depends_on: {{ ref('workday__bronze_promoted') }}
{{ config(
    materialized='view',
    schema='staging',
    tags=['workday', 'silver:class_hr_working_hours']
) }}

SELECT
    tenant_id                 AS insight_tenant_id,
    source_id,
    unique_key,
    Employee_ID               AS source_person_id,
    Work_Email                AS email,
    COALESCE(Display_Name, Work_Email) AS display_name,
    Worker_Type               AS employment_type,
    'workday'                 AS source,
    -- Scheduled_Weekly_Hours is a Workday-delivered field; fall back to a
    -- 40h week when the report column is empty for a worker.
    coalesce(toFloat64OrNull(toString(Scheduled_Weekly_Hours)), 40.0) / 5.0
                              AS working_hours_per_day,
    coalesce(toFloat64OrNull(toString(Scheduled_Weekly_Hours)), 40.0)
                              AS working_hours_per_week,
    _airbyte_extracted_at     AS ingested_at
FROM {{ source('workday', 'workers') }}
WHERE Worker_Status = 'Active'
  AND Employee_ID IS NOT NULL
  AND Work_Email IS NOT NULL
