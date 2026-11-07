{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_teams']
) }}

-- Teams are known only through owner memberships; the most recently updated
-- owner carrying a team decides its name.
WITH memberships AS (
    {% for tbl in hubspot_bronze_tables('owners') %}
    SELECT
        tenant_id,
        source_id,
        team,
        data_source,
        {{ hubspot_record_version() }}                  AS owner_version
    FROM {{ source('bronze_hubspot', tbl) }}
    ARRAY JOIN JSONExtractArrayRaw(coalesce(teams, '[]')) AS team
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
),

src AS (
    SELECT
        tenant_id,
        source_id,
        concat(tenant_id, '-', source_id, '-', JSONExtractString(team, 'id')) AS unique_key,
        JSONExtractString(team, 'id')                   AS team_id,
        JSONExtractString(team, 'name')                 AS name,
        data_source,
        owner_version                                   AS _version
    FROM memberships
)

{{ hubspot_staging_tail('src') }}
