-- depends_on: {{ ref('active_directory__bronze_promoted') }}
-- Bronze → Silver step 1: Active Directory users → class_people
-- Full-refresh source. Maps directory records to the unified person registry.
-- Current-state snapshot: exactly one row per user. `valid_from` records when the
-- directory object was created; there is no version history here. Attribute
-- history lives in active_directory__users_snapshot /
-- active_directory__users_fields_history, which accumulate across syncs.
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
    -- Entity-level grain: bronze `unique_key` is already
    -- `{tenant}-{source}-{objectGUID}`, so silver `class_people` (versionless RMT
    -- ORDER BY unique_key) collapses to one row per user. Do NOT add a version
    -- axis here — that would make every changed record a second
    -- permanently-"current" row (see ADR-0004).
    CAST(coalesce(u.unique_key, '') AS String)      AS unique_key,
    coalesce(u.tenant_id, '')                       AS workspace_id,
    -- person_id resolved in Silver Step 2 via Identity Manager
    CAST(NULL AS Nullable(UUID))                    AS person_id,
    parseDateTimeBestEffortOrNull(u.whenCreated)    AS valid_from,
    'active-directory'                              AS source,
    u.id                                            AS source_person_id,
    u.employeeId                                    AS employee_number,
    u.displayName                                   AS display_name,
    u.givenName                                     AS first_name,
    u.surname                                       AS last_name,
    -- Prefer `mail` as canonical address; fall back to UPN when mail unset.
    coalesce(u.mail, u.userPrincipalName)           AS email,
    u.jobTitle                                      AS job_title,
    -- `department_name` is the org cohort key for this class. There is no
    -- org-unit UUID anywhere in the system (no `org_units` table exists), so
    -- the former always-NULL `org_unit_id Nullable(UUID)` column was dropped.
    -- Downstream `org_unit_id` (insight.people, gold views, the frontend) is a
    -- department NAME string derived from this field.
    u.department                                    AS department_name,
    -- manager_person_id is the resolved unified person ID, not an AD source ID.
    -- Leave it null here; Silver Step 2 (Identity Manager) resolves it from the
    -- parent_id/parent_email identity_inputs signals emitted by
    -- active_directory__manager_identity_inputs.sql.
    -- Nullable(String), not Nullable(UUID): the sibling HR sources project a
    -- String here (ms-entra casts Nullable(String); bamboohr/workday project
    -- their source supervisor-ID columns), and union_by_tag UNION ALLs all four
    -- into silver.class_people — a UUID branch raises Code: 386 NO_COMMON_TYPE
    -- (cpt-dataflow-constraint-staging-class-column-types-match).
    CAST(NULL AS Nullable(String))                  AS manager_person_id,
    -- NULL accountEnabled is genuinely unknown (never 'active' by default —
    -- defaulting to active silently inflates headcount).
    CASE
        WHEN u.accountEnabled IS NOT NULL AND u.accountEnabled THEN 'active'
        WHEN u.accountEnabled IS NOT NULL AND NOT u.accountEnabled THEN 'terminated'
        ELSE 'unknown'
    END                                             AS status,
    -- AD has no employment-type field; default until the BambooHR join in
    -- Silver Step 2 (Identity Manager) supplies the real value.
    'full_time'                                     AS employment_type,
    CAST(NULL AS Nullable(DateTime))                AS hire_date,
    CAST(NULL AS Nullable(DateTime))                AS termination_date,
    CAST(NULL AS Nullable(String))                  AS location,
    CAST(NULL AS Nullable(String))                  AS country,
    CAST(NULL AS Nullable(Float64))                 AS fte,
    CAST(map(
        'sam_account_name',     coalesce(u.sAMAccountName, ''),
        'distinguished_name',   coalesce(u.distinguishedName, '')
    ) AS Map(String, String))                       AS custom_str_attrs,
    CAST(map() AS Map(String, Float64))             AS custom_num_attrs,
    u._airbyte_extracted_at                         AS ingested_at
-- FINAL is mandatory, not defensive: bronze is append-only (a full snapshot per
-- sync) and RMT(_airbyte_extracted_at) only collapses on background merge. A
-- bare read emits every unmerged snapshot row, and the downstream
-- `LIMIT 1 BY unique_key` has no ORDER BY, so the winner is undefined.
-- FINAL makes "latest sync wins" deterministic. See ADR-0001.
FROM {{ source('bronze_active_directory', 'users') }} u FINAL
