-- depends_on: {{ ref('workday__bronze_promoted') }}
-- Bronze → Silver step 1: Workday Workers → class_people
-- Full-refresh source (RaaS report returns current state only).
-- SCD Type 2: valid_from = Last_Functionally_Updated, valid_to = NULL
-- (current-state snapshot). Full SCD history tracking is handled downstream.
-- @cpt-constraint:cpt-dataflow-constraint-staging-class-column-types-match:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['workday', 'silver:class_people']
) }}

SELECT
    tenant_id,
    source_id,
    -- SCD2 grain: per (entity, valid_from). Bronze `unique_key` is at entity
    -- level (`{tenant}-{source}-{employee_id}`); we extend it with valid_from
    -- so silver `class_people` can dedup by a single ORDER BY column.
    CAST(concat(coalesce(unique_key, ''), '-', toString(Last_Functionally_Updated)) AS String) AS unique_key,
    coalesce(tenant_id, '')                                  AS workspace_id,
    -- person_id resolved in Silver Step 2 via Identity Manager
    CAST(NULL AS Nullable(UUID))                             AS person_id,
    parseDateTimeBestEffortOrNull(Last_Functionally_Updated) AS valid_from,
    CAST(NULL AS Nullable(DateTime))                         AS valid_to,
    'workday'                                                AS source,
    Employee_ID                                              AS source_person_id,
    Employee_ID                                              AS employee_number,
    Display_Name                                             AS display_name,
    First_Name                                               AS first_name,
    Last_Name                                                AS last_name,
    Work_Email                                               AS email,
    Business_Title                                           AS job_title,
    -- Workday has no freeform department; the supervisory organization is the
    -- standard org unit every tenant is guaranteed to have.
    Supervisory_Organization                                 AS department_name,
    CAST(NULL AS Nullable(UUID))                             AS org_unit_id,
    Manager_Employee_ID                                      AS manager_person_id,
    multiIf(
        Worker_Status = 'Terminated', 'terminated',
        Worker_Status = 'On Leave',   'on_leave',
        'active'
    )                                                        AS status,
    multiIf(
        Worker_Type = 'Contingent Worker', 'contractor',
        'full_time'
    )                                                        AS employment_type,
    parseDateTimeBestEffortOrNull(Hire_Date)                 AS hire_date,
    parseDateTimeBestEffortOrNull(Termination_Date)          AS termination_date,
    Location                                                 AS location,
    Country                                                  AS country,
    CAST(NULL AS Nullable(Float64))                          AS fte,
    CAST(map(
        'job_profile', coalesce(Job_Profile, ''),
        'worker_type', coalesce(Worker_Type, '')
    ) AS Map(String, String))                                AS custom_str_attrs,
    CAST(map() AS Map(String, Float64))                      AS custom_num_attrs,
    _airbyte_extracted_at                                    AS ingested_at
FROM {{ source('workday', 'workers') }}
