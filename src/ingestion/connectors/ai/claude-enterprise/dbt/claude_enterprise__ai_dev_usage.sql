-- depends_on: {{ ref('claude_enterprise__bronze_promoted') }}
-- Bronze → Silver: Claude Enterprise per-user per-day Code activity.
--
-- Source: bronze_claude_enterprise.claude_enterprise_users — daily per-user
-- snapshot of Claude Enterprise usage including code_* fields.
--
-- Strategy: Enterprise is the canonical source for Claude Code activity
-- (lines_added, sessions, tool_accepted/rejected) for orgs on the Enterprise
-- subscription. Claude Admin's claude_admin_code_usage is no longer tagged
-- for silver:class_ai_dev_usage — Enterprise covers the same activity at
-- the user-day grain plus additional dimensions (chat, cowork, office) that
-- Admin doesn't expose. Admin remains tagged for silver:class_ai_api_usage
-- (token-level metrics that Enterprise doesn't publish).
--
-- Fundamental gap: Enterprise reports `code_lines_added` (AI-accepted lines)
-- but does not expose `total_lines_added` (every keystroke including manual
-- edits). This is the same limitation every assistant-mode AI coder has —
-- Cursor exposes both because it sees keystrokes; Claude Code only sees its
-- own suggestions. Downstream metrics that need a denominator (e.g.,
-- `ai_loc_share_pct = accepted/total`) emit NULL for Enterprise rows; that
-- metric is Cursor-only by design.
--
-- Ingestion shape:
--   • One row per (user_id, date). claude_enterprise_users is a daily
--     snapshot, so we treat it as already aggregated at the (email, day)
--     grain. No SUM/argMax wrapping needed.
--   • Filter: code_session_count > 0 OR code_lines_added > 0
--                                       OR code_tool_accepted_count > 0
--                                       OR code_commit_count > 0
--                                       OR code_pull_request_count > 0
--     Drop rows with no Code activity that day. The commit/PR conditions catch
--     days where Anthropic registers a git event without a session counter
--     (edge case observed in real Enterprise data).

{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['claude-enterprise', 'silver:class_ai_dev_usage']
) }}

SELECT
    tenant_id                                                          AS insight_tenant_id,
    source_id,
    CAST(concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', coalesce(user_id, ''), '-', coalesce(date, '')) AS String)
                                                                       AS unique_key,
    lower(trim(user_email))                                            AS email,
    CAST(NULL AS Nullable(String))                                     AS api_key_id,
    toDate(parseDateTimeBestEffortOrNull(date))                        AS day,
    'claude_code'                                                      AS tool,
    toUInt32(coalesce(code_session_count, 0))                          AS session_count,
    toUInt32(coalesce(code_lines_added, 0))                            AS lines_added,
    toUInt32(coalesce(code_lines_removed, 0))                          AS lines_removed,
    -- Enterprise reports AI-accepted lines only — no view of total user keystrokes.
    -- ai_loc_share_pct downstream filters to tool='cursor' precisely because of this gap.
    CAST(NULL AS Nullable(UInt32))                                     AS total_lines_added,
    CAST(NULL AS Nullable(UInt32))                                     AS total_lines_removed,
    toUInt32(coalesce(code_tool_accepted_count, 0)
           + coalesce(code_tool_rejected_count, 0))                    AS tool_use_offered,
    toUInt32(coalesce(code_tool_accepted_count, 0))                    AS tool_use_accepted,
    -- completions_count := accepted Code tool invocations (Edit/Write/MultiEdit/NotebookEdit)
    toUInt32(coalesce(code_tool_accepted_count, 0))                    AS completions_count,
    -- agent_sessions := no direct Enterprise equivalent. Enterprise's cowork_dispatch_turn_count
    -- represents agentic turns in Cowork (different product surface), not Code agent runs.
    -- Leave NULL until Anthropic exposes a Code-specific agent counter.
    CAST(NULL AS Nullable(UInt32))                                     AS agent_sessions,
    -- chat_requests: Enterprise has no Code-specific chat counter. Its chat_message_count
    -- covers Claude.ai chat turns (a different product surface than Code chat in IDEs/CLI),
    -- so attributing it here would mix surfaces. Leave NULL until Anthropic exposes a
    -- Code-scoped chat counter.
    CAST(NULL AS Nullable(UInt32))                                     AS chat_requests,
    -- Cost is not surfaced per-user in Enterprise; tied to org subscription, not consumption.
    CAST(NULL AS Nullable(UInt32))                                     AS cost_cents,
    -- Enterprise exposes commit and PR counts per user per day via core_metrics.
    toUInt32(coalesce(code_commit_count, 0))                           AS commits_count,
    toUInt32(coalesce(code_pull_request_count, 0))                     AS pull_requests_count,
    -- Full tool-action breakdown (edit/write/multi_edit/notebook_edit accepted+rejected).
    -- Stored as-is from Bronze for downstream analytics without re-aggregation.
    claude_code_metrics_json                                           AS tool_action_breakdown_json,
    'claude_enterprise'                                                AS source,
    'insight_claude_enterprise'                                        AS data_source,
    parseDateTime64BestEffortOrNull(coalesce(collected_at, ''), 3)     AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)                      AS _version
FROM (
    -- Bronze deduplication: bronze_claude_enterprise.claude_enterprise_users
    -- is plain MergeTree (Airbyte default). Re-emitted days under the 3-day
    -- incremental lookback accumulate multiple Bronze rows per (user_id, date)
    -- with different _airbyte_extracted_at. Keep only the latest. Once
    -- promote_bronze_to_rmt is enabled for this connector (ADR-0002), this
    -- LIMIT 1 BY becomes a no-op but stays as defensive depth.
    SELECT *
    FROM {{ source('bronze_claude_enterprise', 'claude_enterprise_users') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY tenant_id, source_id, user_id, date
)
WHERE user_email IS NOT NULL
  AND trim(user_email) != ''
  AND date IS NOT NULL
  AND (coalesce(code_session_count, 0) > 0
       OR coalesce(code_lines_added, 0) > 0
       OR coalesce(code_tool_accepted_count, 0) > 0
       OR coalesce(code_commit_count, 0) > 0
       OR coalesce(code_pull_request_count, 0) > 0)
{% if is_incremental() %}
  AND toDate(parseDateTimeBestEffortOrNull(date)) > (
      SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 3 DAY
      FROM {{ this }}
  )
{% endif %}
