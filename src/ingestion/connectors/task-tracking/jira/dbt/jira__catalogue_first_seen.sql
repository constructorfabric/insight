-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    alias='jira__catalogue_first_seen',
    schema='staging',
    engine='MergeTree()',
    order_by=['insight_source_id'],
    tags=['staging', 'jira']
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
-- count as "old". A source already recorded here is skipped, so the instant
-- survives every later run; only `--full-refresh` re-derives it, from whatever
-- bronze still shows at that moment.

SELECT
    COALESCE(source_id, '')                               AS insight_source_id,
    min(_airbyte_extracted_at)                            AS catalogue_first_sync,
    now64(3)                                              AS recorded_at
FROM {{ source('bronze_jira', 'jira_fields') }}
{% if is_incremental() %}
WHERE COALESCE(source_id, '') NOT IN (SELECT insight_source_id FROM {{ this }})
{% endif %}
GROUP BY insight_source_id
