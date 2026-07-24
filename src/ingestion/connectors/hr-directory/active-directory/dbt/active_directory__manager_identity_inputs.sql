{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['active-directory', 'silver', 'silver:identity_inputs']
) }}

{# Emit parent_email identity signals by resolving AD managerDn → manager email.

   The main identity_inputs model uses `identity_inputs_from_history` which emits
   raw field values. AD's manager is stored as a Distinguished Name (DN), not an
   email — e.g. `CN=John Doe,OU=R&D,...,DC=corp,DC=acronis,DC=com`. The C# seeder's
   org_chart rebuild joins `parent_email` against `email` observations, so we need
   to resolve the DN to the manager's email via a self-join on `distinguishedName`.

   This mirrors how BambooHR emits `supervisorEmail` as `parent_email` and Workday
   emits `Manager_Work_Email` as `parent_email`.
#}

WITH history AS (
    SELECT *
    FROM {{ ref('active_directory__users_fields_history') }}
    {% if is_incremental() %}
    WHERE updated_at > (SELECT max(_synced_at) FROM {{ this }})
    {% endif %}
),

manager_changes AS (
    SELECT
        h.tenant_id,
        h.source_id,
        h.entity_id,
        h.new_value AS manager_dn,
        h.updated_at,
        -- Resolve the manager DN to their primary email
        coalesce(mgr.mail, mgr.userPrincipalName) AS manager_email,
        mgr.id AS manager_source_id
    FROM history h
    LEFT JOIN {{ source('bronze_active_directory', 'users') }} mgr
        ON h.new_value = mgr.distinguishedName
    WHERE h.field_name = 'managerDn'
      AND h.new_value != ''
)

-- parent_email: resolved manager email for org_chart building
SELECT
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        'active-directory', '-',
        coalesce(entity_id, ''), '-',
        'parent_email', '-',
        'UPSERT-',
        toString(toUnixTimestamp64Milli(toDateTime64(updated_at, 3)))
    ) AS String) AS unique_key,
    toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
    toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
    'active-directory' AS insight_source_type,
    entity_id AS source_account_id,
    'parent_email' AS value_type,
    manager_email AS value,
    'bronze_active_directory.users.managerDn' AS value_field_name,
    'UPSERT' AS operation_type,
    toDateTime64(updated_at, 3) AS _synced_at,
    toUnixTimestamp64Milli(toDateTime64(updated_at, 3)) AS _version
FROM manager_changes
WHERE manager_email IS NOT NULL AND manager_email != ''

UNION ALL

-- parent_id: manager's source_person_id for direct parent_person_id resolution
SELECT
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        'active-directory', '-',
        coalesce(entity_id, ''), '-',
        'parent_id', '-',
        'UPSERT-',
        toString(toUnixTimestamp64Milli(toDateTime64(updated_at, 3)))
    ) AS String) AS unique_key,
    toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
    toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
    'active-directory' AS insight_source_type,
    entity_id AS source_account_id,
    'parent_id' AS value_type,
    manager_source_id AS value,
    'bronze_active_directory.users.managerDn' AS value_field_name,
    'UPSERT' AS operation_type,
    toDateTime64(updated_at, 3) AS _synced_at,
    toUnixTimestamp64Milli(toDateTime64(updated_at, 3)) AS _version
FROM manager_changes
WHERE manager_source_id IS NOT NULL AND manager_source_id != ''
