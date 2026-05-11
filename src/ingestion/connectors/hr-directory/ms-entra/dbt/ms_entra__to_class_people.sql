-- depends_on: {{ ref('ms_entra__bronze_promoted') }}
-- Bronze → Silver step 1: MS Entra users → class_people
-- Full-refresh source. Maps directory records to the unified person registry.
-- SCD Type 2: valid_from = createdDateTime, valid_to = NULL (current-state snapshot).
-- Full SCD history is handled downstream via ms_entra__users_fields_history.
{{ config(
    materialized='view',
    schema='staging',
    tags=['ms-entra', 'silver:class_people']
) }}

SELECT
    tenant_id,
    source_id,
    -- SCD2 grain: per (entity, valid_from). Bronze `unique_key` is at entity
    -- level (`{tenant}-{source}-{oid}`); we extend it with valid_from so
    -- silver `class_people` can dedup by a single ORDER BY column.
    CAST(concat(coalesce(unique_key, ''), '-', toString(coalesce(createdDateTime, ''))) AS String) AS unique_key,
    coalesce(tenant_id, '')                         AS workspace_id,
    -- person_id resolved in Silver Step 2 via Identity Manager
    CAST(NULL AS Nullable(UUID))                    AS person_id,
    parseDateTimeBestEffortOrNull(createdDateTime)   AS valid_from,
    CAST(NULL AS Nullable(DateTime))                AS valid_to,
    'ms-entra'                                      AS source,
    id                                              AS source_person_id,
    employeeId                                      AS employee_number,
    displayName                                     AS display_name,
    givenName                                       AS first_name,
    surname                                         AS last_name,
    -- Prefer `mail` as canonical address; fall back to UPN when mail unset
    -- (common for guest/external users).
    coalesce(mail, userPrincipalName)               AS email,
    jobTitle                                        AS job_title,
    department                                      AS department_name,
    CAST(NULL AS Nullable(UUID))                    AS org_unit_id,
    -- Manager relationships are not collected in v1 of the connector;
    -- a future iteration will add `$expand=manager` to populate this.
    CAST(NULL AS Nullable(String))                  AS manager_person_id,
    CASE
        WHEN accountEnabled IS NOT NULL AND accountEnabled THEN 'active'
        WHEN accountEnabled IS NOT NULL AND NOT accountEnabled THEN 'terminated'
        ELSE 'active'
    END                                             AS status,
    -- Entra has no employment-type field; default until the BambooHR join
    -- in Silver Step 2 (Identity Manager) supplies the real value.
    'full_time'                                     AS employment_type,
    CAST(NULL AS Nullable(Date))                    AS hire_date,
    CAST(NULL AS Nullable(Date))                    AS termination_date,
    CAST(NULL AS Nullable(String))                  AS location,
    CAST(NULL AS Nullable(String))                  AS country,
    CAST(NULL AS Nullable(Float64))                 AS fte,
    CAST(map(
        'user_type',          coalesce(userType, ''),
        'sam_account_name',   coalesce(onPremisesSamAccountName, '')
    ) AS Map(String, String))                       AS custom_str_attrs,
    CAST(map() AS Map(String, Float64))             AS custom_num_attrs,
    _airbyte_extracted_at                           AS ingested_at
FROM {{ source('bronze_ms_entra', 'users') }}
