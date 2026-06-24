-- depends_on: {{ ref('active_directory__bronze_promoted') }}
-- Bronze → Silver step 1: Active Directory users → class_people
-- Full-refresh source. Maps directory records to the unified person registry.
-- SCD Type 2: valid_from = whenCreated, valid_to = NULL (current-state snapshot).
-- Full SCD history is handled downstream via active_directory__users_fields_history.
-- Sibling of ms_entra__to_class_people — same column contract, LDAP attributes.
-- @cpt-constraint:cpt-dataflow-constraint-staging-class-column-types-match:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['active-directory', 'silver:class_people']
) }}

SELECT
    u.tenant_id,
    u.source_id,
    -- SCD2 grain: per (entity, valid_from). Bronze `unique_key` is at entity
    -- level (`{tenant}-{source}-{objectGUID}`); extend it with valid_from so
    -- silver `class_people` can dedup by a single ORDER BY column.
    CAST(concat(coalesce(u.unique_key, ''), '-', toString(coalesce(u.whenCreated, ''))) AS String) AS unique_key,
    coalesce(u.tenant_id, '')                       AS workspace_id,
    -- person_id resolved in Silver Step 2 via Identity Manager
    CAST(NULL AS Nullable(UUID))                    AS person_id,
    parseDateTimeBestEffortOrNull(u.whenCreated)    AS valid_from,
    CAST(NULL AS Nullable(DateTime))                AS valid_to,
    'active-directory'                              AS source,
    u.id                                            AS source_person_id,
    u.employeeId                                    AS employee_number,
    u.displayName                                   AS display_name,
    u.givenName                                     AS first_name,
    u.surname                                       AS last_name,
    -- Prefer `mail` as canonical address; fall back to UPN when mail unset.
    coalesce(u.mail, u.userPrincipalName)           AS email,
    u.jobTitle                                      AS job_title,
    u.department                                    AS department_name,
    CAST(NULL AS Nullable(UUID))                    AS org_unit_id,
    -- Resolve managerDn (a Distinguished Name) to the manager's objectGUID
    -- via a self-join on distinguishedName. This mirrors how BambooHR maps
    -- supervisorEId → manager_person_id.
    mgr.id                                          AS manager_person_id,
    CASE
        WHEN u.accountEnabled IS NOT NULL AND u.accountEnabled THEN 'active'
        WHEN u.accountEnabled IS NOT NULL AND NOT u.accountEnabled THEN 'terminated'
        ELSE 'active'
    END                                             AS status,
    -- AD has no employment-type field; default until the BambooHR join in
    -- Silver Step 2 (Identity Manager) supplies the real value.
    'full_time'                                     AS employment_type,
    CAST(NULL AS Nullable(Date))                    AS hire_date,
    CAST(NULL AS Nullable(Date))                    AS termination_date,
    CAST(NULL AS Nullable(String))                  AS location,
    CAST(NULL AS Nullable(String))                  AS country,
    CAST(NULL AS Nullable(Float64))                 AS fte,
    CAST(map(
        'sam_account_name',     coalesce(u.sAMAccountName, ''),
        'distinguished_name',   coalesce(u.distinguishedName, '')
    ) AS Map(String, String))                       AS custom_str_attrs,
    CAST(map() AS Map(String, Float64))             AS custom_num_attrs,
    u._airbyte_extracted_at                         AS ingested_at
FROM {{ source('bronze_active_directory', 'users') }} u
LEFT JOIN {{ source('bronze_active_directory', 'users') }} mgr
    ON u.managerDn = mgr.distinguishedName
