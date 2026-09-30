{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

SELECT coalesce(source_id, '') AS insight_source_id, coalesce(field_id, '') AS field_id,
    coalesce(field_type_id, '') AS field_type,
    JSONExtractRaw({{ youtrack_json('values_json') }}, 'value') AS value,
    JSONExtractString(value, 'id') AS value_id, JSONExtractString(value, 'name') AS value_name,
    toDateTime64(_airbyte_extracted_at, 3) AS observed_at
FROM {{ source('bronze_youtrack', 'youtrack_field_values') }} FINAL
QUALIFY row_number() OVER (PARTITION BY insight_source_id, field_id, value_id ORDER BY observed_at DESC) = 1
