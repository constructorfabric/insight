{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_pipeline_stages']
) }}

WITH pipelines AS (
    {% for tbl in ['deal_pipelines', 'ticket_pipelines'] %}
    SELECT
        tenant_id,
        source_id,
        id                                              AS pipeline_id,
        objectType                                      AS object_type,
        coalesce(archived, false)                       AS pipeline_archived,
        stages,
        data_source,
        _airbyte_extracted_at
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
),

stages AS (
    SELECT
        tenant_id,
        source_id,
        pipeline_id,
        object_type,
        pipeline_archived,
        stage,
        data_source,
        _airbyte_extracted_at
    FROM pipelines
    ARRAY JOIN JSONExtractArrayRaw(coalesce(stages, '[]')) AS stage
),

src AS (
    SELECT
        tenant_id,
        source_id,
        concat(tenant_id, '-', source_id, '-', object_type, '-', JSONExtractString(stage, 'id')) AS unique_key,
        JSONExtractString(stage, 'id')                  AS stage_id,
        pipeline_id,
        object_type,
        JSONExtractString(stage, 'label')               AS label,
        JSONExtract(stage, 'displayOrder', 'Nullable(Int64)') AS display_order,
        toFloat64OrNull(JSONExtractString(stage, 'metadata', 'probability')) AS probability,
        CAST(
            if(JSONHas(stage, 'metadata', 'isClosed'), toUInt8(JSONExtractString(stage, 'metadata', 'isClosed') = 'true'), NULL)
            AS Nullable(UInt8)
        )                                               AS is_closed,
        -- A deal stage is won when it closes at probability 1; ticket stages
        -- carry no probability and stay NULL.
        CAST(
            if(object_type = 'deals', toUInt8(JSONExtractString(stage, 'metadata', 'isClosed') = 'true' AND toFloat64OrNull(JSONExtractString(stage, 'metadata', 'probability')) = 1), NULL)
            AS Nullable(UInt8)
        )                                               AS is_won,
        nullIf(JSONExtractString(stage, 'metadata', 'ticketState'), '') AS ticket_state,
        toUInt8(pipeline_archived OR JSONExtractBool(stage, 'archived')) AS is_deleted,
        parseDateTime64BestEffortOrNull(JSONExtractString(stage, 'createdAt'), 3) AS created_at,
        parseDateTime64BestEffortOrNull(JSONExtractString(stage, 'updatedAt'), 3) AS updated_at,
        data_source,
        toUnixTimestamp64Milli(_airbyte_extracted_at)   AS _version
    FROM stages
)

{{ hubspot_staging_tail('src') }}
