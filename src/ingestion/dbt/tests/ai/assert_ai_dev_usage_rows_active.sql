{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'class_ai_dev_usage rows carry real activity',
        'domain': 'ai',
        'category': 'grain',
        'tier': 'error',
        'remediation': 'The class contract guarantees that a (person, day, tool) row exists only when the person actually used the tool that day — insight.ai_metric_observations derives active_day from row existence. A row here means a staging model in the silver:class_ai_dev_usage tag emitted a zero-activity row (seat/roster entry): fix that model''s emission filter, do not patch consumers.'
    }
) }}
SELECT
    insight_tenant_id,
    email,
    day,
    tool,
    source
FROM {{ ref('class_ai_dev_usage') }}
WHERE coalesce(session_count, 0) = 0
  AND coalesce(conversation_count, 0) = 0
  AND coalesce(lines_added, 0) = 0
  AND coalesce(lines_removed, 0) = 0
  AND coalesce(total_lines_added, 0) = 0
  AND coalesce(total_lines_removed, 0) = 0
  AND coalesce(tool_use_offered, 0) = 0
  AND coalesce(tool_use_accepted, 0) = 0
  AND coalesce(agent_sessions, 0) = 0
  AND coalesce(chat_requests, 0) = 0
  AND coalesce(cost_cents, 0) = 0
  AND coalesce(commits_count, 0) = 0
  AND coalesce(pull_requests_count, 0) = 0
  AND coalesce(prs_with_cc_count, 0) = 0
  AND coalesce(prs_total_count, 0) = 0
