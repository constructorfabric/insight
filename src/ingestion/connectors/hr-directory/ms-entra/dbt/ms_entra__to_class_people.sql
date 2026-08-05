-- depends_on: {{ ref('ms_entra__bronze_promoted') }}
-- Bronze → Silver step 1: MS Entra users → class_people
-- Full-refresh source. Maps directory records to the unified person registry.
-- Current-state snapshot: exactly one row per user. `valid_from` records when
-- the directory object was created; there is no version history here.
-- Attribute history lives in ms_entra__users_snapshot /
-- ms_entra__users_fields_history, which accumulate across syncs.
-- Canonical reference implementation for cpt-dataflow-constraint-staging-class-column-types-match.
-- @cpt-constraint:cpt-dataflow-constraint-staging-class-column-types-match:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['ms-entra', 'silver:class_people']
) }}

SELECT
    tenant_id,
    source_id,
    -- Entity-level grain: bronze `unique_key` is already
    -- `{tenant}-{source}-{oid}`, so silver `class_people` (versionless RMT
    -- ORDER BY unique_key) collapses to one row per user. Do NOT add a version
    -- axis here — that would make every changed record a second
    -- permanently-"current" row (see ADR-0004).
    CAST(coalesce(unique_key, '') AS String)        AS unique_key,
    coalesce(tenant_id, '')                         AS workspace_id,
    -- person_id resolved in Silver Step 2 via Identity Manager
    CAST(NULL AS Nullable(UUID))                    AS person_id,
    parseDateTimeBestEffortOrNull(createdDateTime)   AS valid_from,
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
    -- `department_name` is the org cohort key for this class. There is no
    -- org-unit UUID anywhere in the system (no `org_units` table exists), so
    -- the former always-NULL `org_unit_id Nullable(UUID)` column was dropped.
    -- Downstream `org_unit_id` (insight.people, gold views, the frontend) is a
    -- department NAME string derived from this field.
    department                                      AS department_name,
    -- Manager relationships are not collected in v1 of the connector;
    -- a future iteration will add `$expand=manager` to populate this.
    CAST(NULL AS Nullable(String))                  AS manager_person_id,
    -- NULL accountEnabled is genuinely unknown (never 'active' by default —
    -- defaulting to active silently inflates headcount).
    CASE
        WHEN accountEnabled IS NOT NULL AND accountEnabled THEN 'active'
        WHEN accountEnabled IS NOT NULL AND NOT accountEnabled THEN 'terminated'
        ELSE 'unknown'
    END                                             AS status,
    -- Entra has no employment-type field; default until the BambooHR join
    -- in Silver Step 2 (Identity Manager) supplies the real value.
    'full_time'                                     AS employment_type,
    CAST(NULL AS Nullable(DateTime))                AS hire_date,
    CAST(NULL AS Nullable(DateTime))                AS termination_date,
    CAST(NULL AS Nullable(String))                  AS location,
    CAST(NULL AS Nullable(String))                  AS country,
    CAST(NULL AS Nullable(Float64))                 AS fte,
    CAST(map(
        'user_type',          coalesce(userType, ''),
        'sam_account_name',   coalesce(onPremisesSamAccountName, '')
    ) AS Map(String, String))                       AS custom_str_attrs,
    CAST(map() AS Map(String, Float64))             AS custom_num_attrs,
    _airbyte_extracted_at                           AS ingested_at
-- FINAL is mandatory, not defensive: bronze is append-only (a full snapshot per
-- sync) and RMT(_airbyte_extracted_at) only collapses on background merge. A
-- bare read emits every unmerged snapshot row, and the downstream
-- `LIMIT 1 BY unique_key` has no ORDER BY, so the winner is undefined.
-- FINAL makes "latest sync wins" deterministic. See ADR-0001.
FROM {{ source('bronze_ms_entra', 'users') }} FINAL
