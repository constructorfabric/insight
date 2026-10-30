{{ config(
    materialized='incremental',
    incremental_strategy='append',
    full_refresh=false,
    alias='jira__catalogue_first_seen',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    tags=['staging', 'jira', 'history']
) }}

-- When the connector first read the field catalogue, per source — written once
-- and kept. See `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md`
-- §3.2: it is the line between a field deleted before the connector ever
-- looked and a field whose metadata has not arrived.
--
-- It cannot be recomputed on demand. `bronze_jira.jira_fields` is a
-- ReplacingMergeTree versioned by `_airbyte_extracted_at`, so once its parts
-- merge each field keeps only its LATEST extraction and `min()` over the table
-- drifts towards the most recent sync — an ever-younger set of events would
-- count as "old". A source already recorded here is skipped, and
-- `full_refresh=false` keeps even `--full-refresh` from re-deriving it.

WITH first_syncs AS (
    SELECT
        tenant_id,
        CAST(source_id AS String)                         AS insight_source_id,
        min(_airbyte_extracted_at)                        AS catalogue_first_sync,
        now64(3)                                          AS recorded_at
    FROM {{ source('bronze_jira', 'jira_fields') }}
    GROUP BY tenant_id, insight_source_id
)

SELECT
    concat(ifNull(tenant_id, ''), '-', insight_source_id)             AS unique_key,
    tenant_id,
    insight_source_id,
    catalogue_first_sync,
    recorded_at,
    -- INVARIANT: the earliest first sync carries the highest version, so
    -- merging two rows of one source keeps the boundary that was seen first.
    toUInt64(4102444800000 - toUnixTimestamp64Milli(catalogue_first_sync)) AS _version
FROM first_syncs
{% if is_incremental() %}
WHERE unique_key NOT IN (SELECT unique_key FROM {{ this }})
{% endif %}
