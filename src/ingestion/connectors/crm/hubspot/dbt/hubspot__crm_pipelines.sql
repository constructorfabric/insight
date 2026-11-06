{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_pipelines']
) }}

-- A stage edit need not move the pipeline's updatedAt, so the poll time
-- versions the row; a pipeline the latest poll no longer returns is deleted.
WITH polls AS (
    {{ hubspot_pipeline_polls() }}
),

src AS (
    SELECT
        tenant_id,
        source_id,
        concat(tenant_id, '-', source_id, '-', object_type, '-', pipeline_id) AS unique_key,
        pipeline_id,
        object_type,
        label,
        CAST(displayOrder AS Nullable(Int64))           AS display_order,
        toUInt8(coalesce(archived, false) OR {{ hubspot_absent_from_poll('_airbyte_extracted_at', 'polled_at') }}) AS is_deleted,
        createdAt                                       AS created_at,
        updatedAt                                       AS updated_at,
        data_source,
        toUnixTimestamp64Milli(polled_at)               AS _version
    FROM polls
)

{{ hubspot_staging_tail('src') }}
