{{ config(materialized='incremental', full_refresh=false, schema='staging', tags=['youtrack', 'staging'], engine=insight_engine('ReplacingMergeTree', '_version'), order_by=['unique_key'], settings={'allow_nullable_key': 1}, incremental_strategy='append') }}

SELECT concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', coalesce(id, ''), '-',
              toString(_airbyte_extracted_at)) AS unique_key,
    coalesce(source_id, '') AS insight_source_id, coalesce(id, '') AS field_id,
    coalesce(name, '') AS field_name, coalesce(field_type_id, '') AS field_type,
    toDateTime64(_airbyte_extracted_at, 3) AS observed_at,
    toUInt64(toUnixTimestamp64Milli(_airbyte_extracted_at)) AS _version
FROM {{ source('bronze_youtrack', 'youtrack_custom_fields') }} FINAL
WHERE id IS NOT NULL
