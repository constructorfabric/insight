-- Phase 1 (Initial Seed): Cursor members → person.persons
-- One-time seed. Creates person records from Cursor Bronze data.
-- Idempotent: skips cursor members whose email already exists in persons.
-- Source: docs/domain/identity-resolution/specs/DECOMPOSITION.md §2.1
--
-- Prerequisite: person.persons table created by scripts/migrations/20260408000000_init-identity.sql
-- Run: dbt run --select seed_persons_from_cursor

{{ config(
    materialized='incremental',
    unique_key='id',
    order_by=['id'],
    settings={'allow_nullable_key': 1},
    schema='person',
    tags=['identity:seed', 'person']
) }}

SELECT
    generateUUIDv7()                                        AS id,
    -- TEMPORARY: sipHash128 derives UUID from string tenant_id until tenants table exists
    UUIDNumToString(sipHash128(coalesce(tenant_id, '')))             AS insight_tenant_id,
    coalesce(name, '')                                      AS display_name,
    'cursor'                                                AS display_name_source,
    CASE
        WHEN isRemoved = true THEN 'inactive'
        ELSE 'active'
    END                                                     AS status,
    lower(trim(coalesce(email, '')))                        AS email,
    'cursor'                                                AS email_source,
    ''                                                      AS username,
    ''                                                      AS username_source,
    coalesce(role, '')                                      AS role,
    'cursor'                                                AS role_source,
    toUUID('00000000-0000-0000-0000-000000000000')          AS manager_person_id,
    ''                                                      AS manager_person_id_source,
    toUUID('00000000-0000-0000-0000-000000000000')          AS org_unit_id,
    ''                                                      AS org_unit_id_source,
    ''                                                      AS location,
    ''                                                      AS location_source,
    -- completeness_score = non-empty golden attributes / 7 total
    -- (display_name, email, username, role, manager_person_id, org_unit_id, location)
    -- see: docs/domain/person/specs/DESIGN.md §3.7 Table: persons
    (if(name IS NOT NULL AND name != '', 1, 0)
     + if(email IS NOT NULL AND email != '', 1, 0)
     + if(role IS NOT NULL AND role != '', 1, 0)
    ) / 7.0                                                 AS completeness_score,
    'clean'                                                 AS conflict_status,
    now64(3)                                                AS created_at,
    now64(3)                                                AS updated_at,
    0                                                       AS is_deleted,
    toUnixTimestamp64Milli(now64())                          AS _version
FROM {{ source('bronze_cursor', 'cursor_members') }} cm
WHERE cm.email IS NOT NULL AND trim(cm.email) != ''
QUALIFY row_number() OVER (PARTITION BY lower(trim(email)), coalesce(tenant_id, '') ORDER BY _airbyte_extracted_at DESC) = 1
{% if is_incremental() %}
  AND NOT EXISTS (
      SELECT 1 FROM {{ this }} ex
      WHERE lower(ex.email) = lower(trim(cm.email))
        AND ex.insight_tenant_id = UUIDNumToString(sipHash128(coalesce(cm.tenant_id, '')))
        AND ex.is_deleted = 0
  )
{% endif %}
