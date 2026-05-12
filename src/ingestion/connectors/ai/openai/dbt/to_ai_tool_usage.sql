-- Bronze → Silver step 1: OpenAI completions usage → class_ai_tool_usage
-- Maps per-user, per-model, per-project daily completion metrics.
-- bucket_start_time is Unix seconds — converted to date for report_date.
{{ config(materialized='incremental', unique_key='unique_id', tags=['openai']) }}

SELECT
    tenant_id,
    source_id                                       AS insight_source_id,
    concat(
        toString(bucket_start_time), '|',
        coalesce(project_id, ''), '|',
        coalesce(model, ''), '|',
        coalesce(user_id, '')
    )                                               AS unique_id,
    toDate(fromUnixTimestamp(
        CAST(bucket_start_time AS UInt32)
    ))                                              AS report_date,
    user_id,
    project_id,
    model,
    input_tokens,
    output_tokens,
    input_cached_tokens,
    input_audio_tokens,
    output_audio_tokens,
    num_model_requests,
    coalesce(batch, false)                          AS is_batch,
    service_tier,
    NULL                                            AS person_id,
    'openai'                                        AS provider,
    'openai_api'                                    AS client,
    'insight_openai'                                AS data_source
FROM {{ source('bronze_openai', 'usage_completions') }}
{% if is_incremental() %}
WHERE toDate(fromUnixTimestamp(CAST(bucket_start_time AS UInt32)))
    > (SELECT max(report_date) FROM {{ this }})
{% endif %}
