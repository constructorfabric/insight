-- Bronze → Silver step 1: Claude Admin programmatic API usage → class_ai_api_usage
--
-- Source: bronze_claude_admin.claude_admin_messages_usage — daily token usage
-- from /v1/organizations/usage_report/messages, grouped by
-- (date, model, api_key_id, workspace_id, service_tier, context_window).
--
-- email is always NULL here: programmatic API calls authenticate via API keys
-- and Anthropic cannot attribute consumption to individual users at request
-- time. Per-user attribution comes via Silver Step 2 (Identity Resolution) by
-- treating api_key_id as an identity key — when the organisation provisions
-- one API key per developer (common practice), api_key_id → person_id is a
-- clean 1:1 mapping. Fallback per-user attribution via Enterprise engagement
-- (claude_enterprise_users.chat_*) in claude_enterprise__ai_assistant_usage.
--
-- channel = 'api' for all rows produced here.
--
-- Resilience: if bronze_claude_admin.claude_admin_messages_usage is absent
-- (stream disabled at the connection level, e.g. to work around an ongoing
-- rate-limit issue at the Anthropic Admin API layer, or a fresh deploy before
-- the first sync), emit an empty structurally-typed relation instead of
-- failing the run. Downstream class_ai_api_usage still compiles correctly
-- (Admin is currently the sole contributor to this Silver class).
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['claude-admin', 'silver:class_ai_api_usage']
) }}

{%- set messages_usage = adapter.get_relation(database=none, schema='bronze_claude_admin', identifier='claude_admin_messages_usage') -%}

{%- if not messages_usage %}
-- messages_usage Bronze table missing; emit empty result so the pipeline
-- still compiles and materialises downstream silver views without error.
SELECT
    CAST(NULL AS Nullable(String))                  AS insight_tenant_id,
    CAST(NULL AS Nullable(String))                  AS source_id,
    CAST('' AS String)                              AS unique_key,
    CAST(NULL AS Nullable(String))                  AS email,
    CAST(NULL AS Nullable(String))                  AS api_key_id,
    CAST(NULL AS Nullable(String))                  AS workspace_id,
    CAST(NULL AS Nullable(Date))                    AS day,
    CAST('anthropic' AS String)                     AS provider,
    CAST('api' AS String)                           AS channel,
    CAST(NULL AS Nullable(UInt64))                  AS input_tokens,
    CAST(NULL AS Nullable(UInt64))                  AS output_tokens,
    CAST(NULL AS Nullable(UInt64))                  AS cache_read_tokens,
    CAST(NULL AS Nullable(UInt64))                  AS cache_creation_tokens,
    CAST(NULL AS Nullable(Decimal(18, 4)))          AS cost_amount,
    CAST(NULL AS Nullable(String))                  AS cost_currency,
    CAST('claude_admin' AS String)                  AS source,
    CAST('insight_claude_admin' AS String)          AS data_source,
    CAST(NULL AS Nullable(DateTime64(3)))           AS collected_at,
    CAST(0 AS UInt64)                               AS _version
WHERE 1 = 0
{%- else %}

SELECT
    tenant_id                                       AS insight_tenant_id,
    insight_source_id                               AS source_id,
    -- coalesce Nullable Bronze inputs to '' before concat so unique_key is a
    -- non-nullable String. ClickHouse MergeTree requires non-nullable keys in
    -- ORDER BY (allow_nullable_key=0 by default, and we'd rather not rely on
    -- that session setting).
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(insight_source_id, ''), '-',
        'ws:', coalesce(workspace_id, '__null__'), '-',
        toString(toDate(date)), '-',
        'anthropic-api-',
        coalesce(model, '__null__'), '-',
        coalesce(api_key_id, '__null__'), '-',
        coalesce(service_tier, '__null__'), '-',
        coalesce(context_window, '__null__')
    ) AS String)                                    AS unique_key,
    CAST(NULL AS Nullable(String))                  AS email,
    api_key_id,
    workspace_id,
    toDate(date)                                    AS day,
    'anthropic'                                     AS provider,
    'api'                                           AS channel,
    toUInt64(coalesce(uncached_input_tokens, 0)
           + coalesce(cache_read_tokens, 0)
           + coalesce(cache_creation_5m_tokens, 0)
           + coalesce(cache_creation_1h_tokens, 0)) AS input_tokens,
    toUInt64(coalesce(output_tokens, 0))            AS output_tokens,
    toUInt64(coalesce(cache_read_tokens, 0))        AS cache_read_tokens,
    toUInt64(coalesce(cache_creation_5m_tokens, 0)
           + coalesce(cache_creation_1h_tokens, 0)) AS cache_creation_tokens,
    CAST(NULL AS Nullable(Decimal(18, 4)))          AS cost_amount,
    CAST(NULL AS Nullable(String))                  AS cost_currency,
    'claude_admin'                                  AS source,
    'insight_claude_admin'                          AS data_source,
    CAST(parseDateTime64BestEffortOrNull(coalesce(collected_at, ''), 3) AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)   AS _version
FROM {{ source('bronze_claude_admin', 'claude_admin_messages_usage') }}
{% if is_incremental() %}
WHERE toDate(date) > (
    SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 3 DAY
    FROM {{ this }}
)
{% endif %}
{%- endif %}
