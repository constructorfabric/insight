{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_tickets']
) }}

-- INVARIANT: columns up to `_version` mirror silver.dim_support_ticket in
-- name, order and type, so the two classes can be unioned positionally.
WITH src AS (
    {% for tbl in hubspot_bronze_tables('tickets') %}
    SELECT
        tenant_id,
        source_id                                       AS insight_source_id,
        unique_key,
        'hubspot'                                       AS data_source,
        id                                              AS source_ticket_id,
        properties_subject                              AS subject,
        CAST(if(properties_hs_is_closed = 'true', 'closed', 'open') AS Nullable(String)) AS status_canonical,
        multiIf(
            lower(coalesce(properties_hs_ticket_priority, '')) = 'medium', 'normal',
            lower(coalesce(properties_hs_ticket_priority, ''))
        )                                               AS priority_canonical,
        lower(coalesce(properties_hs_ticket_category, '')) AS type_canonical,
        properties_hubspot_owner_id                     AS assignee_source_id,
        CAST(NULL AS Nullable(String))                  AS assignee_person_key,
        properties_hubspot_team_id                      AS group_source_id,
        {{ hubspot_first_association('associations_contacts') }}  AS requester_source_id,
        {{ hubspot_first_association('associations_companies') }} AS org_source_id,
        toDateTime(createdAt)                           AS created_at,
        toDateTime(updatedAt)                           AS updated_at,
        toDateTime(parseDateTime64BestEffortOrNull(toString(properties_closed_date), 3)) AS solved_at,
        toDateTime(coalesce(collected_at, _airbyte_extracted_at)) AS collected_at,
        {{ hubspot_record_version() }}                  AS _version,
        properties_hs_pipeline                          AS pipeline_id,
        properties_hs_pipeline_stage                    AS stage_id,
        properties_hs_created_by_user_id                AS created_by_user_id,
        {{ hubspot_first_association('associations_deals') }} AS deal_id,
        properties_content                              AS content,
        properties_hs_resolution                        AS resolution,
        properties_source_type                          AS source_type,
        {{ hubspot_deletion_columns() }},
        {{ hubspot_properties_map() }}                  AS properties
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)

{{ hubspot_staging_tail('src', source_key='insight_source_id') }}
