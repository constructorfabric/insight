{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_contacts']
) }}

WITH src AS (
    {% for tbl in hubspot_bronze_tables('contacts') %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS contact_id,
        properties_email                                AS email,
        properties_firstname                            AS first_name,
        properties_lastname                             AS last_name,
        properties_hubspot_owner_id                     AS owner_id,
        {{ hubspot_first_association('associations_companies') }} AS account_id,
        properties_lifecyclestage                       AS lifecycle_stage,
        toJSONString(map(
            'phone',            coalesce(toString(properties_phone), ''),
            'city',             coalesce(toString(properties_city), ''),
            'state',            coalesce(toString(properties_state), ''),
            'country',          coalesce(toString(properties_country), ''),
            'jobtitle',         coalesce(toString(properties_jobtitle), ''),
            'hs_lead_status',   coalesce(toString(properties_hs_lead_status), ''),
            'hs_analytics_source', coalesce(toString(properties_hs_analytics_source), ''),
            'archived',         toString(coalesce(archived, false))
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
