{{ config(materialized='incremental', full_refresh=false, schema='staging', tags=['youtrack', 'staging'], engine='ReplacingMergeTree(_version)', order_by=['unique_key'], settings={'allow_nullable_key': 1}, incremental_strategy='append') }}

SELECT candidate.*
FROM (
    SELECT concat(tenant_id, '-', insight_source_id, '-', issue_id, '-', toString(observed_at)) AS unique_key,
        *, toUInt64(toUnixTimestamp64Milli(observed_at)) AS _version
    FROM {{ ref('youtrack__issues') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'insight_source_id']) }}
