{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'class_ai_assistant_usage rows carry real activity',
        'domain': 'ai',
        'category': 'grain',
        'tier': 'error',
        'remediation': 'The class contract guarantees that a (person, day, tool, surface) row exists only when the person actually used the surface that day — insight.ai_metric_observations derives active_day from row existence. A row here means a staging model in the silver:class_ai_assistant_usage tag emitted a zero-activity row: fix that model''s emission filter, do not patch consumers.'
    }
) }}
SELECT
    insight_tenant_id,
    email,
    day,
    tool,
    surface,
    source
FROM {{ ref('class_ai_assistant_usage') }}
WHERE coalesce(session_count, 0) = 0
  AND coalesce(conversation_count, 0) = 0
  AND coalesce(message_count, 0) = 0
  AND coalesce(action_count, 0) = 0
  AND coalesce(files_uploaded_count, 0) = 0
  AND coalesce(artifacts_created_count, 0) = 0
  AND coalesce(projects_created_count, 0) = 0
  AND coalesce(projects_used_count, 0) = 0
  AND coalesce(skills_used_count, 0) = 0
  AND coalesce(connectors_used_count, 0) = 0
  AND coalesce(thinking_message_count, 0) = 0
  AND coalesce(dispatch_turn_count, 0) = 0
  AND coalesce(search_count, 0) = 0
  AND coalesce(cost_cents, 0) = 0
