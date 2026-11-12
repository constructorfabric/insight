{{ config(
    materialized='table',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['mock']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    id,
    value,
    checksum,
    last_updated_at,
    scenario_revision,
    toUnixTimestamp64Milli(_airbyte_extracted_at) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_mock', 'mock_events') }} FINAL
