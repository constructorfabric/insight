{{ config(
    materialized='table',
    engine=insight_engine('ReplacingMergeTree'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['hubspot']
) }}

-- One row per owner across the live and archived Bronze tables, so the
-- identity snapshot sees an archive as a change of the same account. The
-- owner id keys the chain because deals and engagements reference it.
WITH owners AS (
    {% for tbl in hubspot_bronze_tables('owners') %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS owner_id,
        lower(trim(coalesce(email, '')))                AS email,
        trim(concat(coalesce(firstName, ''), ' ', coalesce(lastName, ''))) AS display_name,
        toUInt8(coalesce(archived, false))              AS is_archived,
        {{ hubspot_record_version() }}                  AS _version
    FROM {{ source('bronze_hubspot', tbl) }} FINAL
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)

SELECT
    tenant_id,
    source_id,
    unique_key,
    owner_id,
    email,
    display_name,
    is_archived
FROM owners
WHERE owner_id != ''
ORDER BY _version DESC
LIMIT 1 BY unique_key
