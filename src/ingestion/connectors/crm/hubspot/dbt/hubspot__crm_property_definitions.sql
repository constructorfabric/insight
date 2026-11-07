{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_property_definitions']
) }}

-- HubSpot-defined properties often carry no updatedAt, so the extraction
-- time versions the row; the connector re-reads every definition each sync.
WITH src AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        objectType                                      AS object_type,
        name,
        label,
        description,
        groupName                                       AS group_name,
        type,
        fieldType                                       AS field_type,
        toUInt8(NOT coalesce(hubspotDefined, false))    AS is_custom,
        toUInt8(coalesce(calculated, false))            AS is_calculated,
        toUInt8(coalesce(hidden, false))                AS is_hidden,
        CAST(
            arrayMap(
                option -> (JSONExtractString(option, 'value'), JSONExtractString(option, 'label')),
                JSONExtractArrayRaw(coalesce(options, '[]'))
            ),
            'Map(String, String)'
        )                                               AS options,
        toUInt8(coalesce(archived, false))              AS is_deleted,
        createdAt                                       AS created_at,
        updatedAt                                       AS updated_at,
        data_source,
        toUnixTimestamp64Milli(_airbyte_extracted_at)   AS _version
    FROM {{ source('bronze_hubspot', 'property_definitions') }}
)

{{ hubspot_staging_tail('src') }}
