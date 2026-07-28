{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'source_key', 'measure_key', 'entity_id', 'metric_date', 'record_id'],
    schema='insight',
    alias='ai_metric_evidence',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 1610612736,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
) }}


WITH
ai_dev_usage_source AS (
    SELECT
        insight_tenant_id AS tenant_id,
        lower(email) AS entity_id,
        day AS metric_date,
        CAST(
            [tuple('tool', tool, {{ ai_tool_label('tool') }})]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions,
        conversation_count,
        lines_added,
        lines_removed,
        tool_use_offered,
        tool_use_accepted,
        cost_cents
    FROM {{ ref('class_ai_dev_usage') }}
    WHERE email IS NOT NULL
      AND email != ''
),
ai_assistant_usage_source AS (
    SELECT
        insight_tenant_id AS tenant_id,
        lower(email) AS entity_id,
        day AS metric_date,
        surface,
        CAST(
            [tuple('tool', tool, {{ ai_tool_label('tool') }})]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions,
        CAST(
            [
                tuple('tool', tool, {{ ai_tool_label('tool') }}),
                tuple('surface', surface, {{ ai_surface_label('surface') }})
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_surface_dimensions,
        conversation_count,
        message_count,
        action_count,
        cost_cents
    FROM {{ ref('class_ai_assistant_usage') }}
    WHERE email IS NOT NULL
      AND email != ''
),
measure_observations AS (
    {{ sum_measure('accepted_lines', 'ai_dev_usage_source', 'lines_added', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('removed_lines', 'ai_dev_usage_source', 'lines_removed', 'tool_dimensions') }}

    UNION ALL

    {{ presence_measure('active_day', ['ai_dev_usage_source', 'ai_assistant_usage_source']) }}

    UNION ALL

    {{ sum_measure('cost_usd', 'ai_dev_usage_source', 'cost_cents / 100', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('cost_usd', 'ai_assistant_usage_source', 'cost_cents / 100', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('accepted_edit_actions', 'ai_dev_usage_source', 'tool_use_accepted', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('tool_use_offered', 'ai_dev_usage_source', 'tool_use_offered', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('dev_conversations', 'ai_dev_usage_source', 'conversation_count', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('assistant_messages', 'ai_assistant_usage_source', 'message_count', 'tool_surface_dimensions') }}

    UNION ALL

    {{ sum_measure('assistant_actions', 'ai_assistant_usage_source', 'action_count', 'tool_surface_dimensions') }}

    UNION ALL

    {{ sum_measure('chat_assistant_conversations', 'ai_assistant_usage_source', 'conversation_count', 'tool_surface_dimensions', where="surface = 'chat'") }}
),
evidence_summaries AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        measure_key,
        toNullable(sum(value)) AS value,
        dimensions
    FROM measure_observations
    GROUP BY tenant_id, entity_id, metric_date, measure_key, dimensions
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'ai_usage' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    concat(
        toString(metric_date),
        ':',
        measure_key,
        ':',
        hex(sipHash128(toString(arrayMap(d -> tuple(d.1, d.2), dimensions))))
    ) AS record_id,
    measure_key AS record_kind,
    if(measure_key = 'active_day', 'derived_population', 'source_summary') AS granularity,
    replaceAll(measure_key, '_', ' ') AS record_label,
    value AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions,
    CAST(map() AS Map(String, String)) AS details
FROM evidence_summaries
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
