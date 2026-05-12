-- Phase 1 (Initial Seed): Cursor members → identity.aliases
-- One-time seed. Creates alias records linking cursor identities to persons.
-- Idempotent: skips aliases that already exist (by value_type + value + source).
-- Source: docs/domain/identity-resolution/specs/DECOMPOSITION.md §2.1
--
-- Prerequisite:
--   identity.aliases table created by scripts/migrations/20260408000000_init-identity.sql
--   person.persons populated by seed_persons_from_cursor
-- Run: dbt run --select seed_aliases_from_cursor

{{ config(
    materialized='incremental',
    unique_key='id',
    order_by=['id'],
    settings={'allow_nullable_key': 1},
    schema='identity',
    tags=['identity:seed', 'aliases']
) }}

-- Each cursor member emits up to 3 alias rows: email, id, display_name.
-- `id` carries source_account_id as the ADR-0002 canonical binding observation
-- (replaces the former `platform_id`, which was always equal to source_account_id).
-- Join on email to resolve person_id from the seed_persons_from_cursor output.

WITH source AS (
    SELECT
        cm.id                                                       AS source_account_id,
        cm.name,
        cm.email,
        cm.tenant_id,
        p.id                                                        AS person_id,
        p.insight_tenant_id
    FROM {{ source('bronze_cursor', 'cursor_members') }} cm
    INNER JOIN {{ ref('seed_persons_from_cursor') }} p
        ON lower(trim(cm.email)) = lower(p.email)
        AND UUIDNumToString(sipHash128(coalesce(cm.tenant_id, ''))) = p.insight_tenant_id  -- TEMPORARY: until tenants table
    WHERE cm.email IS NOT NULL AND cm.email != ''
),

-- Unpivot: one row per alias type
aliases AS (
    -- email
    SELECT
        generateUUIDv7()                                            AS id,
        insight_tenant_id,
        person_id,
        'email'                                                     AS value_type,
        lower(trim(email))                                          AS value,
        'bronze_cursor.cursor_members.email'                        AS value_field_name,
        toUUID('00000000-0000-0000-0000-000000000000')              AS insight_source_id,
        'cursor'                                                    AS insight_source_type,
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
        'bronze_cursor.cursor_members.id',
        toUUID('00000000-0000-0000-0000-000000000000'),
        'cursor',
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
        'bronze_cursor.cursor_members.name',
        toUUID('00000000-0000-0000-0000-000000000000'),
        'cursor',
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

SELECT a.*, toUnixTimestamp64Milli(now64()) AS _version FROM aliases a
{% if is_incremental() %}
LEFT ANTI JOIN {{ this }} existing
    ON  a.value_type          = existing.value_type
    AND a.value               = existing.value
    AND a.insight_tenant_id   = existing.insight_tenant_id
    AND a.insight_source_type = existing.insight_source_type
    AND a.source_account_id   = existing.source_account_id
    AND existing.is_deleted   = 0
{% endif %}
