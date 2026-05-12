-- Bronze → Silver step 1: OpenAI costs → class_ai_cost
-- Maps per-line-item, per-project daily cost data.
-- bucket_start_time is Unix seconds — converted to date for report_date.
{{ config(materialized='incremental', unique_key='unique_id', tags=['openai']) }}

SELECT
    tenant_id,
    source_id                                       AS insight_source_id,
    concat(
        toString(bucket_start_time), '|',
        coalesce(line_item, ''), '|',
        coalesce(project_id, '')
    )                                               AS unique_id,
    toDate(fromUnixTimestamp(
        CAST(bucket_start_time AS UInt32)
    ))                                              AS report_date,
    line_item,
    project_id,
    amount_value,
    amount_currency,
    'openai'                                        AS provider,
    'insight_openai'                                AS data_source
FROM {{ source('bronze_openai', 'costs') }}
{% if is_incremental() %}
WHERE toDate(fromUnixTimestamp(CAST(bucket_start_time AS UInt32)))
    > (SELECT max(report_date) FROM {{ this }})
{% endif %}
