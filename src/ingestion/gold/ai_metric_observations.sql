{{ config(
    materialized='view',
    schema='insight',
    alias='ai_metric_observations',
    tags=['gold']
) }}

-- Source measure observations for the unified metrics runtime. Reads only
-- class-contract fields: activity is row existence (the class contract
-- guarantees rows exist only for real activity — see silver/ai/schema.yml),
-- display labels come from tool_label / surface_label, and conversation
-- semantics come from data presence (conversation_count is NULL for sources
-- without a conversation concept). No vendor-specific columns, tool names, or
-- label mappings may appear in this model. Every measure is emitted through
-- the shape macros in macros/metric_observation_measures.sql; filter
-- predicates may reference only class-contract dimension values.

WITH
ai_dev_usage_source AS (
    SELECT
        insight_tenant_id AS tenant_id,
        lower(email) AS entity_id,
        day AS metric_date,
        coalesce(nullIf(tool, ''), '__unknown__') AS tool_value,
        if(
            coalesce(nullIf(tool, ''), '__unknown__') = '__unknown__',
            'Unknown',
            coalesce(nullIf(tool_label, ''), tool)
        ) AS tool_label_value,
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
        coalesce(nullIf(tool, ''), '__unknown__') AS tool_value,
        if(
            coalesce(nullIf(tool, ''), '__unknown__') = '__unknown__',
            'Unknown',
            coalesce(nullIf(tool_label, ''), tool)
        ) AS tool_label_value,
        coalesce(nullIf(surface, ''), '__unknown__') AS surface_value,
        if(
            coalesce(nullIf(surface, ''), '__unknown__') = '__unknown__',
            'Unknown',
            coalesce(nullIf(surface_label, ''), surface)
        ) AS surface_label_value,
        conversation_count,
        message_count,
        action_count,
        cost_cents
    FROM {{ ref('class_ai_assistant_usage') }}
    WHERE email IS NOT NULL
      AND email != ''
),
ai_dev_usage_dimensions AS (
    SELECT
        *,
        CAST(
            [tuple('tool', tool_value, tool_label_value)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions
    FROM ai_dev_usage_source
),
ai_assistant_usage_dimensions AS (
    SELECT
        *,
        CAST(
            [tuple('tool', tool_value, tool_label_value)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions,
        CAST(
            [
                tuple('tool', tool_value, tool_label_value),
                tuple('surface', surface_value, surface_label_value)
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_surface_dimensions
    FROM ai_assistant_usage_source
),
measure_observations AS (
    {{ sum_measure('accepted_lines', 'ai_dev_usage_dimensions', 'lines_added', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('removed_lines', 'ai_dev_usage_dimensions', 'lines_removed', 'tool_dimensions') }}

    UNION ALL

    {{ presence_measure('active_day', ['ai_dev_usage_source', 'ai_assistant_usage_source']) }}

    UNION ALL

    {{ sum_measure('cost_usd', 'ai_dev_usage_dimensions', 'cost_cents / 100', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('cost_usd', 'ai_assistant_usage_dimensions', 'cost_cents / 100', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('accepted_edit_actions', 'ai_dev_usage_dimensions', 'tool_use_accepted', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('tool_use_offered', 'ai_dev_usage_dimensions', 'tool_use_offered', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('dev_conversations', 'ai_dev_usage_dimensions', 'conversation_count', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('assistant_messages', 'ai_assistant_usage_dimensions', 'message_count', 'tool_surface_dimensions') }}

    UNION ALL

    {{ sum_measure('assistant_actions', 'ai_assistant_usage_dimensions', 'action_count', 'tool_surface_dimensions') }}

    UNION ALL

    {{ sum_measure('chat_assistant_conversations', 'ai_assistant_usage_dimensions', 'conversation_count', 'tool_surface_dimensions', where="surface_value = 'chat'") }}
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'ai_usage' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    value,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions
FROM measure_observations
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
