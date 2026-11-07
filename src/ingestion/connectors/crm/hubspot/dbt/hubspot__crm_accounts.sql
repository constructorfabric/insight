{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_accounts']
) }}

WITH src AS (
    {% for tbl in hubspot_bronze_tables('companies') %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS account_id,
        properties_name                                 AS name,
        properties_domain                               AS domain,
        properties_industry                             AS industry,
        properties_hubspot_owner_id                     AS owner_id,
        -- HubSpot has no native parent-account hierarchy in v3.
        CAST(NULL AS Nullable(String))                  AS parent_account_id,
        toJSONString(map(
            'city',              coalesce(toString(properties_city), ''),
            'state',             coalesce(toString(properties_state), ''),
            'country',           coalesce(toString(properties_country), ''),
            'numberofemployees', coalesce(toString(properties_numberofemployees), ''),
            'annualrevenue',     coalesce(toString(properties_annualrevenue), ''),
            'archived',          toString(coalesce(archived, false))
        ))                                              AS metadata,
        createdAt                                       AS created_at,
        updatedAt                                       AS updated_at,
        data_source,
        {{ hubspot_record_version() }}                  AS _version,
        {{ hubspot_deletion_columns() }},
        {{ hubspot_properties_map() }}                  AS properties
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)

{{ hubspot_staging_tail('src') }}
