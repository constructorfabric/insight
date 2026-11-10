{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_leads']
) }}

WITH src AS (
    {% for tbl in hubspot_bronze_tables('leads') %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS lead_id,
        properties_hs_lead_name                         AS name,
        properties_hs_pipeline                          AS pipeline_id,
        properties_hs_pipeline_stage                    AS stage_id,
        properties_hubspot_owner_id                     AS owner_id,
        properties_hs_created_by_user_id                AS created_by_user_id,
        properties_hs_lead_type                         AS lead_type,
        properties_hs_lead_label                        AS lead_label,
        coalesce(
            nullIf(properties_hs_primary_contact_id, ''),
            {{ hubspot_first_association('associations_contacts') }}
        )                                               AS contact_id,
        coalesce(
            nullIf(properties_hs_primary_company_id, ''),
            {{ hubspot_first_association('associations_companies') }}
        )                                               AS account_id,
        properties_hs_lead_disqualification_reason      AS disqualification_reason,
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
