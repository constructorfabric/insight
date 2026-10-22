{{ config(
    materialized='table',
    engine=insight_engine('ReplacingMergeTree'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['youtrack']
) }}

-- `project_json` stays raw: the task-tracking Silver layer owns the mapping
-- from a vendor project to the class contract, and it does not exist yet.

SELECT
    tenant_id,
    source_id,
    unique_key,
    id AS project_id,
    name,
    shortName AS short_key,
    description,
    archived,
    project_json,
    observed_at
FROM (
    -- Read-time dedup of append-only RMT bronze (ADR-0001): keep the latest
    -- extract per unique_key so re-delivered rows never duplicate downstream.
    SELECT * FROM {{ source('bronze_youtrack', 'youtrack_projects') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key
)
