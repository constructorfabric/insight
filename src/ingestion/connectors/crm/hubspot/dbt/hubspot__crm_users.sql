{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_users']
) }}

-- Two HubSpot identifiers to track per owner:
--   * `user_id`    — `owners.id`     — used in `properties_hubspot_owner_id`
--                                       on deals / engagements (record-owner side).
--   * `hs_user_id` — `owners.userId` — used in `hs_created_by_user_id` on
--                                       engagements + deals (who-logged-it side).
-- Both need to resolve to a single canonical rep (`email`), so we expose
-- them as parallel columns; Silver gold-side joins pick whichever applies.
WITH src AS (
    {% for tbl in hubspot_bronze_tables('owners') %}
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
        toNullable(toInt64(NOT coalesce(archived, false)))          AS is_active,
        toJSONString(map(
            'userId',   coalesce(toString(userId), ''),
            'archived', toString(coalesce(archived, false))
        ))                                              AS metadata,
        collected_at,
        data_source,
        {{ hubspot_record_version() }}                  AS _version,
        nullIf(JSONExtractString(
            arrayFirst(team -> JSONExtractBool(team, 'primary'), JSONExtractArrayRaw(coalesce(teams, '[]'))),
            'id'
        ), '')                                          AS primary_team_id,
        arrayMap(team -> JSONExtractString(team, 'id'), JSONExtractArrayRaw(coalesce(teams, '[]'))) AS team_ids
    FROM {{ source('bronze_hubspot', tbl) }}
    -- Silver class_crm_users requires email NOT NULL for identity resolution.
    -- HubSpot Owners for deactivated internal users can lack an email.
    WHERE email IS NOT NULL AND email != ''
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)

{{ hubspot_staging_tail('src') }}
