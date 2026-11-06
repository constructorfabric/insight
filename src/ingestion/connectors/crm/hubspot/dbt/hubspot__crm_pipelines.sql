{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_pipelines']
) }}

-- The connector re-reads every pipeline each sync and a stage edit need not
-- move the pipeline's updatedAt, so the extraction time versions the row.
WITH src AS (
    {% for tbl in ['deal_pipelines', 'ticket_pipelines'] %}
    SELECT
        tenant_id,
        source_id,
        concat(tenant_id, '-', source_id, '-', objectType, '-', id) AS unique_key,
        id                                              AS pipeline_id,
        objectType                                      AS object_type,
        label,
        CAST(displayOrder AS Nullable(Int64))           AS display_order,
        toUInt8(coalesce(archived, false))              AS is_deleted,
        createdAt                                       AS created_at,
        updatedAt                                       AS updated_at,
        data_source,
        toUnixTimestamp64Milli(_airbyte_extracted_at)   AS _version
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)

{{ hubspot_staging_tail('src') }}
