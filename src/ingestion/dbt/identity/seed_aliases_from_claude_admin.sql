-- Phase 1 (Initial Seed): Claude Admin users → identity.aliases
-- Idempotent: skips aliases that already exist (by value_type + value + source).
-- Dedup: takes latest row per email by _airbyte_extracted_at.
-- Each user produces up to 3 aliases: email, id, display_name.
-- `id` carries source_account_id as the ADR-0002 canonical binding observation
-- (replaces the former `platform_id`, which was always equal to source_account_id).
-- Source: docs/domain/identity-resolution/specs/DECOMPOSITION.md §2.1
--
-- Prerequisite:
--   identity.aliases table created by scripts/migrations/20260408000000_init-identity.sql
--   person.persons populated (by seed_persons_from_cursor or seed_persons_from_claude_admin)
-- Run: dbt run --select seed_aliases_from_claude_admin
--
-- Test manually: http://localhost:30123/play  (user: default, password: clickhouse_local)
--   or:          http://localhost:8123/play

{{ config(
    materialized='incremental',
    unique_key='id',
    order_by=['id'],
    settings={'allow_nullable_key': 1},
    schema='identity',
    tags=['identity:seed', 'aliases']
) }}

WITH latest AS (
    SELECT id AS source_id, email, name, tenant_id
    FROM {{ source('bronze_claude_admin', 'claude_admin_users') }}
    WHERE email IS NOT NULL AND email != ''
    QUALIFY row_number() OVER (PARTITION BY lower(trim(email)), coalesce(tenant_id, '') ORDER BY _airbyte_extracted_at DESC) = 1
),

source AS (
    SELECT
        l.source_id                                                 AS source_account_id,
        l.name,
        l.email,
        p.id                                                        AS person_id,
        p.insight_tenant_id
    FROM latest l
    INNER JOIN person.persons p ON lower(trim(l.email)) = lower(p.email)
        AND UUIDNumToString(sipHash128(coalesce(l.tenant_id, ''))) = p.insight_tenant_id  -- TEMPORARY: until tenants table
),

new_aliases AS (
    -- email
    SELECT
        generateUUIDv7()                                            AS id,
        insight_tenant_id,
        person_id,
        'email'                                                     AS value_type,
        lower(trim(email))                                          AS value,
        'bronze_claude_admin.claude_admin_users.email'              AS value_field_name,
        toUUID('00000000-0000-0000-0000-000000000000')              AS insight_source_id,
        'claude_admin'                                              AS insight_source_type,
        source_account_id,
        toFloat32(1.0)                                              AS confidence,
        toUInt8(1)                                                  AS is_active,
        now64(3)                                                    AS effective_from,
        toDateTime64('1970-01-01 00:00:00.000', 3, 'UTC')          AS effective_to,
        now64(3)                                                    AS first_observed_at,
        now64(3)                                                    AS last_observed_at,
        now64(3)                                                    AS created_at,
        now64(3)                                                    AS updated_at,
        toUInt8(0)                                                  AS is_deleted
    FROM source
    WHERE email IS NOT NULL AND email != ''

    UNION ALL

    -- id (binding observation per ADR-0002; value = source_account_id)
    SELECT
        generateUUIDv7(),
        insight_tenant_id,
        person_id,
        'id',
        trim(source_account_id),
        'bronze_claude_admin.claude_admin_users.id',
        toUUID('00000000-0000-0000-0000-000000000000'),
        'claude_admin',
        source_account_id,
        toFloat32(1.0),
        toUInt8(1),
        now64(3),
        toDateTime64('1970-01-01 00:00:00.000', 3, 'UTC'),
        now64(3),
        now64(3),
        now64(3),
        now64(3),
        toUInt8(0)
    FROM source
    WHERE source_account_id IS NOT NULL AND source_account_id != ''

    UNION ALL

    -- display_name
    SELECT
        generateUUIDv7(),
        insight_tenant_id,
        person_id,
        'display_name',
        trim(name),
        'bronze_claude_admin.claude_admin_users.name',
        toUUID('00000000-0000-0000-0000-000000000000'),
        'claude_admin',
        source_account_id,
        toFloat32(1.0),
        toUInt8(1),
        now64(3),
        toDateTime64('1970-01-01 00:00:00.000', 3, 'UTC'),
        now64(3),
        now64(3),
        now64(3),
        now64(3),
        toUInt8(0)
    FROM source
    WHERE name IS NOT NULL AND name != ''
)

SELECT na.*, toUnixTimestamp64Milli(now64()) AS _version FROM new_aliases na
{% if is_incremental() %}
LEFT ANTI JOIN {{ this }} existing
    ON  na.value_type          = existing.value_type
    AND na.value               = existing.value
    AND na.insight_tenant_id   = existing.insight_tenant_id
    AND na.source_account_id   = existing.source_account_id
    AND existing.insight_source_type = 'claude_admin'
    AND existing.is_deleted    = 0
{% endif %}
