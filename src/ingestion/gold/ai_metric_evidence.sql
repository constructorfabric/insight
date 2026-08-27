{{ metric_evidence_table() }}

-- Keyed by the source identity through `normalized_email()`, not by person:
-- the analytics runtime resolves through `identity.person_map` while it serves.
-- An unresolvable row stays and starts counting the moment it resolves.
SELECT
    src.tenant_id,
    src.source_key,
    src.entity_type,
    {{ normalized_email('src.entity_id') }} AS entity_id,
    -- No account-keyed facts here; '' leaves the account join unmatched.
    '' AS account_source_type,
    '' AS account_source_id,
    '' AS account_id,
    src.metric_date,
    src.observed_at,
    src.measure_key,
    src.record_id,
    src.record_kind,
    src.granularity,
    src.record_label,
    src.contribution,
    src.subject_key,
    src.dimensions,
    src.details
FROM (


WITH
ai_dev_usage_source AS (
    SELECT
        insight_tenant_id AS tenant_id,
        {{ normalized_email('email') }} AS entity_id,
        day AS metric_date,
        CAST(
            [
                tuple('tool', tool, {{ ai_tool_label('tool') }}),
                -- Seat state as of the last read, not as of metric_date: the
                -- sources restate it for every day they re-read. 'unknown'
                -- where the source has no seat lifecycle concept.
                tuple('seat_status', coalesce(seat_status, 'unknown'), CAST(NULL AS Nullable(String)))
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions,
        conversation_count,
        lines_added,
        lines_removed,
        tool_use_offered,
        tool_use_accepted,
        cost_cents,
        prs_with_cc_count,
        prs_total_count
    FROM {{ ref('class_ai_dev_usage') }}
    WHERE email IS NOT NULL
      AND email != ''
),
ai_assistant_usage_source AS (
    SELECT
        insight_tenant_id AS tenant_id,
        {{ normalized_email('email') }} AS entity_id,
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

    {{ sum_measure('prs_with_assistant', 'ai_dev_usage_source', 'prs_with_cc_count', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('prs_total', 'ai_dev_usage_source', 'prs_total_count', 'tool_dimensions') }}

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
) AS src
