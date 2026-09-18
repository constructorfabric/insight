{{ config(
    materialized='table',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    schema='staging',
    tags=['compass']
) }}

-- The JSON columns (labels, links, event sources, dependency edges) stay raw:
-- flattening them is a silver decision, deferred with silver itself (SPEC.md 7).

SELECT
    tenant_id,
    source_id,
    unique_key,
    data_source,
    component_id,
    name,
    slug,
    component_type,
    description,
    component_url,
    owner_team_id,
    labels,
    links,
    event_sources,
    relationships,
    relationships_truncated,
    collected_at
FROM (
    -- Read-time dedup of append-only RMT bronze (ADR-0001): keep the latest
    -- extract per unique_key so re-delivered rows never duplicate downstream.
    SELECT * FROM {{ source('bronze_compass', 'components') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key
)
