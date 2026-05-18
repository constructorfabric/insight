{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_users']
) }}

-- Live (`owners`) and archived (`owners_archived`) are sibling Bronze tables;
-- ReplacingMergeTree on `unique_key` dedups, with `_version = greatest(updatedAt, archivedAt)`
-- so an archive event always outranks the prior live update. The archived
-- sibling is only synced when Airbyte is configured to backfill deleted
-- records — guard the UNION with adapter.get_relation so absent archived
-- tables don't break the build.
--
-- Two HubSpot identifiers to track per owner:
--   * `user_id`    — `owners.id`     — used in `properties_hubspot_owner_id`
--                                       on deals / engagements (record-owner side).
--   * `hs_user_id` — `owners.userId` — used in `hs_created_by_user_id` on
--                                       engagements + deals (who-logged-it side).
-- Both need to resolve to a single canonical rep (`email`), so we expose
-- them as parallel columns; Silver gold-side joins pick whichever applies.
-- Schema derived from the dbt source so a tenant-prefixed
-- `bronze_hubspot_<tenant>` rename doesn't silently drop the archived arm.
{%- set bronze_schema = source('bronze_hubspot', 'owners').schema -%}
{%- set bronze_tables = ['owners'] -%}
{%- if adapter.get_relation(database=none, schema=bronze_schema, identifier='owners_archived') -%}
  {%- do bronze_tables.append('owners_archived') -%}
{%- endif %}

WITH src AS (
    {% for tbl in bronze_tables %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS user_id,
        -- `bronze_hubspot.owners.userId` is Nullable(Int64); cast to
        -- String so it joins cleanly with `created_by_user_id` (String)
        -- on deal/engagement records.
        toString(userId)                                AS hs_user_id,
        email                                           AS email,
        firstName                                       AS first_name,
        lastName                                        AS last_name,
        -- HubSpot Owners API exposes no title/department; Silver requires
        -- the columns to exist so emit explicit NULLs.
        CAST(NULL AS Nullable(String))                  AS title,
        CAST(NULL AS Nullable(String))                  AS department,
        toInt64(NOT coalesce(archived, false))          AS is_active,
        toJSONString(map(
            'userId',   coalesce(toString(userId), ''),
            'archived', toString(coalesce(archived, false))
        ))                                              AS metadata,
        collected_at,
        data_source,
        greatest(
            coalesce(toUnixTimestamp64Milli(updatedAt), 0),
            coalesce(toUnixTimestamp64Milli(archivedAt), 0)
        )                                               AS _version
    FROM {{ source('bronze_hubspot', tbl) }}
    -- Silver class_crm_users requires email NOT NULL for identity resolution.
    -- HubSpot Owners for deactivated internal users can lack an email.
    WHERE email IS NOT NULL AND email != ''
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)
{% if is_incremental() %}
SELECT src.*
FROM src
LEFT JOIN (
    SELECT tenant_id, source_id, max(_version) AS hwm
    FROM {{ this }}
    GROUP BY tenant_id, source_id
) w
  ON w.tenant_id = src.tenant_id AND w.source_id = src.source_id
WHERE src._version > coalesce(w.hwm, 0)
{% else %}
SELECT * FROM src
{% endif %}
