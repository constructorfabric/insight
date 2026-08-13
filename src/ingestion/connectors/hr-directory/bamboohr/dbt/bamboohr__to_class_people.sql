-- depends_on: {{ ref('bamboohr__bronze_promoted') }}
-- Bronze → Silver step 1: BambooHR Employees → class_people
-- Full-refresh source. Maps employee records to unified person registry.
-- Current-state snapshot: exactly one row per employee. `valid_from` records
-- when the source last changed the record; there is no version history here.
-- HR attribute history lives in `bamboohr__employees_snapshot` /
-- `bamboohr__employees_fields_history`, which accumulate across syncs
-- (this model is rebuilt in full every run and cannot retain history).
-- @cpt-constraint:cpt-dataflow-constraint-staging-class-column-types-match:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['bamboohr', 'silver:class_people']
) }}

SELECT
    tenant_id,
    source_id,
    -- Entity-level grain: bronze `unique_key` is already
    -- `{tenant}-{source}-{employee_id}`, so silver `class_people` (versionless
    -- RMT ORDER BY unique_key) collapses to one row per employee. Do NOT add a
    -- version axis here — that would make every changed record a second
    -- permanently-"current" row (see ADR-0004).
    CAST(coalesce(unique_key, '') AS String)        AS unique_key,
    coalesce(tenant_id, '')                         AS workspace_id,
    -- person_id resolved in Silver Step 2 via Identity Manager
    CAST(NULL AS Nullable(UUID))                    AS person_id,
    parseDateTimeBestEffortOrNull(lastChanged)      AS valid_from,
    'bamboohr'                                      AS source,
    id                                              AS source_person_id,
    employeeNumber                                  AS employee_number,
    displayName                                     AS display_name,
    firstName                                       AS first_name,
    lastName                                        AS last_name,
    workEmail                                       AS email,
    jobTitle                                        AS job_title,
    department                                      AS department_name,
    CAST(NULL AS Nullable(UUID))                    AS org_unit_id,
    supervisorEId                                   AS manager_person_id,
    -- Never default to 'active': a record that is neither explicitly Active nor
    -- explicitly Terminated (e.g. status='' with employmentHistoryStatus
    -- 'Third party') would silently inflate headcount.
    CASE
        WHEN status = 'Active' THEN 'active'
        WHEN employmentHistoryStatus = 'Terminated' THEN 'terminated'
        ELSE 'unknown'
    END                                             AS status,
    'full_time'                                     AS employment_type,
    parseDateTimeBestEffortOrNull(hireDate)          AS hire_date,
    parseDateTimeBestEffortOrNull(terminationDate)   AS termination_date,
    location                                        AS location,
    country                                         AS country,
    CAST(NULL AS Nullable(Float64))                 AS fte,
    CAST(map('division', coalesce(division, '')) AS Map(String, String))
                                                    AS custom_str_attrs,
    CAST(map() AS Map(String, Float64))             AS custom_num_attrs,
    _airbyte_extracted_at                           AS ingested_at
-- FINAL is mandatory, not defensive: bronze is append-only (a full snapshot per
-- sync) and RMT(_airbyte_extracted_at) only collapses on background merge. A
-- bare read emits every unmerged snapshot row, and the downstream
-- `LIMIT 1 BY unique_key` has no ORDER BY — with max_threads=1 it demonstrably
-- picks the STALE row. FINAL makes "latest sync wins" deterministic. See ADR-0001.
FROM {{ source('bamboohr', 'employees') }} FINAL
