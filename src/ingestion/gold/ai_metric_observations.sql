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
-- without a conversation concept). Discriminator and label columns are
-- non-null non-empty by the class contract (enforced by silver schema tests);
-- this model consumes them as-is. No vendor-specific columns, tool names, or
-- label mappings may appear in this model. Every measure is emitted through
-- the shape macros in macros/metric_observation_measures.sql; filter
-- predicates may reference only class-contract dimension values.

WITH
ai_dev_usage_source AS (
    SELECT
        insight_tenant_id AS tenant_id,
        lower(email) AS entity_id,
        day AS metric_date,
        CAST(
            [tuple('tool', tool, tool_label)]
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
            [tuple('tool', tool, tool_label)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions,
        CAST(
            [
                tuple('tool', tool, tool_label),
                tuple('surface', surface, surface_label)
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
