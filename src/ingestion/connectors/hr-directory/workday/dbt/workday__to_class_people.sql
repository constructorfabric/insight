-- depends_on: {{ ref('workday__bronze_promoted') }}
-- Bronze → Silver step 1: Workday Workers → class_people
-- Full-refresh source (RaaS report returns current state only).
-- Current-state snapshot: exactly one row per worker. `valid_from` records when
-- the source last changed the record; there is no version history here.
-- Attribute history lives in workday__workers_snapshot /
-- workday__workers_fields_history, which accumulate across syncs.
-- @cpt-constraint:cpt-dataflow-constraint-staging-class-column-types-match:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['workday', 'silver:class_people']
) }}

SELECT
    tenant_id,
    source_id,
    -- Entity-level grain: bronze `unique_key` is already
    -- `{tenant}-{source}-{employee_id}`, so silver `class_people` (versionless
    -- RMT ORDER BY unique_key) collapses to one row per worker. Do NOT add a
    -- version axis here — that would make every changed record a second
    -- permanently-"current" row (see ADR-0004).
    CAST(coalesce(unique_key, '') AS String)                  AS unique_key,
    coalesce(tenant_id, '')                                  AS workspace_id,
    -- person_id resolved in Silver Step 2 via Identity Manager
    CAST(NULL AS Nullable(UUID))                             AS person_id,
    parseDateTimeBestEffortOrNull(Last_Functionally_Updated) AS valid_from,
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
    -- `department_name` is the org cohort key for this class. There is no
    -- org-unit UUID anywhere in the system (no `org_units` table exists), so
    -- the former always-NULL `org_unit_id Nullable(UUID)` column was dropped.
    -- Downstream `org_unit_id` (insight.people, gold views, the frontend) is a
    -- department NAME string derived from this field.
    Supervisory_Organization                                 AS department_name,
    Manager_Employee_ID                                      AS manager_person_id,
    -- Worker_Status ∈ {Active, On Leave, Terminated} is contract-normative (a
    -- tenant emitting other vocabulary must normalise it in the RaaS report).
    -- Anything outside it is 'unknown', never 'active' by default — defaulting
    -- to active silently inflates headcount.
    multiIf(
        Worker_Status = 'Active',     'active',
        Worker_Status = 'Terminated', 'terminated',
        Worker_Status = 'On Leave',   'on_leave',
        'unknown'
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
-- FINAL is mandatory, not defensive: bronze is append-only (a full snapshot per
-- sync) and RMT(_airbyte_extracted_at) only collapses on background merge. A
-- bare read emits every unmerged snapshot row, and the downstream
-- `LIMIT 1 BY unique_key` has no ORDER BY, so the winner is undefined.
-- FINAL makes "latest sync wins" deterministic. See ADR-0001.
FROM {{ source('workday', 'workers') }} FINAL
