-- Phase 1 (Initial Seed): Cursor members → identity.identity_inputs
-- One-time seed. Writes raw alias observations from Cursor Bronze data.
-- Raw values preserved — normalization applied at read time by downstream consumers.
-- Idempotent: skips rows that already exist (by source + value_type + value + account).
-- Source: docs/domain/identity-resolution/specs/DECOMPOSITION.md §2.1
--
-- Run: dbt run --select seed_identity_inputs_from_cursor
--
-- NOTE: schema='staging' is intentional. Unlike persons/aliases which write
-- directly to canonical tables, identity_inputs uses a multi-source union
-- pattern: each source writes to staging.*, then identity.identity_inputs
-- (table) aggregates them via union_by_tag('silver:identity_inputs').
-- Consistent with bamboohr/zoom connector models that also target staging.

{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['identity:seed', 'silver', 'silver:identity_inputs']
) }}

-- Each cursor member emits up to 3 observation rows: email, id, display_name.
-- `id` carries source_account_id as the ADR-0002 canonical binding observation
-- (replaces the former `platform_id`, which was always equal to source_account_id).
-- Column set matches identity_inputs_from_history macro output.
-- TEMPORARY: insight_tenant_id derived via sipHash128 until tenants table exists.

WITH source AS (
    SELECT
        cm.id                                                       AS source_account_id,
        cm.name,
        cm.email,
        cm.tenant_id
    FROM {{ source('bronze_cursor', 'cursor_members') }} cm
),

observations AS (
    -- email
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, ''))))        AS insight_tenant_id,
        toUUID('00000000-0000-0000-0000-000000000000')                      AS insight_source_id,
        'cursor'                                                            AS insight_source_type,
        source_account_id,
        'email'                                                             AS value_type,
        email                                                               AS value,
        'bronze_cursor.cursor_members.email'                                AS value_field_name,
        'UPSERT'                                                            AS operation_type,
        now64(3)                                                            AS _synced_at
    FROM source
    WHERE email IS NOT NULL AND email != ''

    UNION ALL

    -- id (binding observation per ADR-0002; value = source_account_id)
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))),
        toUUID('00000000-0000-0000-0000-000000000000'),
        'cursor',
        source_account_id,
        'id',
        source_account_id,
        'bronze_cursor.cursor_members.id',
        'UPSERT',
        now64(3)
    FROM source
    WHERE source_account_id IS NOT NULL AND source_account_id != ''

    UNION ALL

    -- display_name
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))),
        toUUID('00000000-0000-0000-0000-000000000000'),
        'cursor',
        source_account_id,
        'display_name',
        name,
        'bronze_cursor.cursor_members.name',
        'UPSERT',
        now64(3)
    FROM source
    WHERE name IS NOT NULL AND name != ''
)

SELECT
    CAST(concat(
        toString(o.insight_tenant_id), '-',
        o.insight_source_type, '-',
        coalesce(o.source_account_id, ''), '-',
        o.value_type, '-',
        o.operation_type, '-',
        toString(toUnixTimestamp64Milli(o._synced_at))
    ) AS String) AS unique_key,
    o.*,
    toUnixTimestamp64Milli(o._synced_at) AS _version
FROM observations o
{% if is_incremental() %}
LEFT ANTI JOIN {{ this }} existing
    ON  o.value_type                 = existing.value_type
    AND o.value                      = existing.value
    AND o.source_account_id          = existing.source_account_id
    AND existing.insight_source_type = 'cursor'
    AND existing.insight_tenant_id   = o.insight_tenant_id
{% endif %}
