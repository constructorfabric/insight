-- depends_on: {{ ref('chatgpt_team__bronze_promoted') }}
-- Bronze → Silver step 1: ChatGPT Team per-user per-day Codex usage → class_ai_dev_usage.
--
-- Source: bronze_chatgpt_team.chatgpt_team_codex_user_daily — daily aggregate
-- pulled via the customer-deployed chatgpt-team-proxy from chatgpt.com's
-- /backend-api/wham/analytics/usage-leaderboard. One row per (email, date).
--
-- Column contract MUST match the other class_ai_dev_usage sources (cursor,
-- claude_team, claude_enterprise, copilot) — union_by_tag does positional
-- UNION ALL. Keep column order/names/types identical to claude_team__ai_dev_usage.
--
-- Mapping notes:
--   tool='codex'                  — dev-tool discriminator (cf. 'claude_code', 'cursor').
--   session_count ← n_threads     — a Codex thread is the closest analogue to a coding session.
--   lines_added ← lines_added     — AI-accepted lines (from code_attribution.lines_of_code.added).
--   cost_cents ← NULL             — `credits` are Codex usage credits, not a currency amount.
--   Codex-only counters (credits, n_turns, text_tokens, current_streak) are
--   preserved in tool_action_breakdown_json so nothing is lost.
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['chatgpt-team', 'silver:class_ai_dev_usage']
) }}

SELECT
    tenant_id                                           AS insight_tenant_id,
    source_id,
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(source_id, ''), '-',
        lower(trim(coalesce(email, ''))), '-',
        coalesce(date, '')
    ) AS String)                                        AS unique_key,
    lower(trim(email))                                  AS email,
    -- Session-based auth (operator chatgpt.com session); users keyed by email.
    CAST(NULL AS Nullable(String))                      AS api_key_id,
    toDate(date)                                        AS day,
    'codex'                                             AS tool,
    'Codex'                                             AS tool_label,
    -- Codex threads ≈ coding sessions. Non-nullable UInt32 per contract.
    toUInt32(coalesce(toUInt32OrNull(toString(n_threads)), 0))        AS session_count,
    toUInt32OrNull(toString(n_threads))                               AS conversation_count,
    toUInt32(coalesce(toUInt32OrNull(toString(lines_added)), 0))      AS lines_added,
    -- Codex does not surface AI-removed lines / total keystrokes.
    CAST(NULL AS Nullable(UInt32))                      AS lines_removed,
    CAST(NULL AS Nullable(UInt32))                      AS total_lines_added,
    CAST(NULL AS Nullable(UInt32))                      AS total_lines_removed,
    -- Inline-completion offered/accepted not exposed by the leaderboard endpoint.
    CAST(NULL AS Nullable(UInt32))                      AS tool_use_offered,
    CAST(NULL AS Nullable(UInt32))                      AS tool_use_accepted,
    CAST(NULL AS Nullable(UInt32))                      AS agent_sessions,
    CAST(NULL AS Nullable(UInt32))                      AS chat_requests,
    -- `credits` are usage credits, not currency → cost_cents NULL (kept in JSON).
    CAST(NULL AS Nullable(UInt32))                      AS cost_cents,
    CAST(NULL AS Nullable(UInt32))                      AS commits_count,
    CAST(NULL AS Nullable(UInt32))                      AS pull_requests_count,
    CAST(NULL AS Nullable(UInt32))                      AS prs_with_cc_count,
    CAST(NULL AS Nullable(UInt32))                      AS prs_total_count,
    -- Codex-specific counters not in the shared contract — preserved here.
    CAST(toJSONString(map(
        'credits',        toString(coalesce(credits, 0)),
        'n_turns',        toString(coalesce(toUInt32OrNull(toString(n_turns)), 0)),
        'text_tokens',    toString(coalesce(toUInt64OrNull(toString(text_tokens)), 0)),
        'current_streak', toString(coalesce(toUInt32OrNull(toString(current_streak)), 0))
    )) AS Nullable(String))                             AS tool_action_breakdown_json,
    'chatgpt_team'                                      AS source,
    data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version
FROM (
    -- Bronze dedup: keep the latest extract per (email, date). Defensive depth —
    -- becomes a no-op once promote_bronze_to_rmt merges (ADR-0002), but guards
    -- against duplicate raw rows from multiple sync attempts.
    SELECT *
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_codex_user_daily') }}
    ORDER BY _airbyte_extracted_at DESC
    -- Dedup on the SAME normalized key the unique_key uses (lower(trim(email))),
    -- else two case-variant spellings of one address both survive and then
    -- collide on unique_key (unique-test failure).
    LIMIT 1 BY tenant_id, source_id, lower(trim(email)), date
)
WHERE email IS NOT NULL
  AND trim(email) != ''
  AND date IS NOT NULL
  -- Only emit rows with real Codex activity so codex_active (the DAU marker)
  -- is not inflated by zero-usage users (mirrors the chat model's filter).
  AND (
        coalesce(credits, 0) > 0
     OR coalesce(toUInt32OrNull(toString(n_turns)),   0) > 0
     OR coalesce(toUInt32OrNull(toString(n_threads)), 0) > 0
     OR coalesce(toUInt32OrNull(toString(lines_added)), 0) > 0
  )
{% if is_incremental() %}
  -- Empty-table guard. Over an empty `this` (the e2e rig resets staging between
  -- tests) `max(day)` is the Date epoch (1970-01-01) and `- INTERVAL 3 DAY`
  -- underflows the Date range, wrapping to ~2149-06-04 — which filters out every
  -- row and leaves the model empty. Short-circuit when empty so the full set is
  -- (re)loaded. Mirrors the cursor / claude_team / m365__collab_* guard.
  AND (
    (SELECT count() FROM {{ this }}) = 0
    OR toDate(date) > (
        SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 3 DAY
        FROM {{ this }}
    )
  )
{% endif %}
