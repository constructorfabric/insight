-- depends_on: {{ ref('github_copilot__bronze_promoted') }}
-- Bronze → Silver: GitHub Copilot per-user per-day Code activity.
--
-- Sources:
--   bronze_github_copilot.copilot_user_metrics  → daily per-user usage stats
--   bronze_github_copilot.copilot_seats         → user_login → user_email bridge
--
-- The Copilot v2 metrics API returns user identity as `user_login` (GitHub
-- handle), not as an email. The `copilot_seats` endpoint is the authoritative
-- bridge that surfaces `user_email` per `user_login` for active seats. We
-- LEFT JOIN it here and drop rows with no resolvable email — same identity
-- contract as Cursor (which JOINs cursor_members) and Claude Enterprise
-- (which has email directly in the daily table).
--
-- NULL-email policy: drop rows where the seat join produces no email. Causes:
--   1. The user holds a seat but their GitHub email is private (the v2 seats
--      endpoint omits `email` for users who haven't made it public).
--   2. The user used Copilot but has been since unassigned (rare; usually
--      surfaces as `pending_cancellation_date != NULL` on the seat row).
-- These are tracked under issue #285 as OQ-COP-1; the drop is a known data
-- gap, not a defect of this staging.
--
-- Field mapping rules (per gist proposal — AI providers metrics matrix):
--   loc_added_sum                       → lines_added
--   loc_deleted_sum                     → lines_removed
--   code_generation_activity_count      → tool_use_offered  (proxy: each
--                                                            generation event
--                                                            ≈ one offered
--                                                            suggestion; the
--                                                            v2 API does not
--                                                            split offered
--                                                            from rejected)
--   code_acceptance_activity_count      → tool_use_accepted, completions_count
--   used_agent  (boolean)               → agent_sessions = 1 if true else NULL
--   used_chat   (boolean)               → chat_requests  = 1 if true else NULL
--   used_cli    (boolean)               → packed into tool_action_breakdown_json
--   total_lines_added/removed           → NULL  (Copilot reports only AI-
--                                                accepted lines, no view of
--                                                manual keystrokes — same gap
--                                                as Claude Code/Enterprise)
--   commits_count, pull_requests_count  → NULL  (per-user PR/commit
--                                                attribution is org-level
--                                                only in Copilot — see
--                                                copilot_org_metrics.pull_requests)
--   cost_cents                          → NULL  (Copilot is per-seat
--                                                subscription, not metered)
--
-- Activity filter: drop rows where the user wasn't active at all that day.
--   Active iff: any of (code_acceptance_activity_count,
--                       code_generation_activity_count,
--                       loc_added_sum,
--                       user_initiated_interaction_count) > 0
--               OR any of (used_chat, used_agent, used_cli) is true.

{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github-copilot', 'silver:class_ai_dev_usage']
) }}

WITH metrics AS (
    -- Defensive bronze dedup: bronze tables are RMT-promoted by
    -- github_copilot__bronze_promoted, but during the first run after a
    -- fresh sync there can still be multiple rows per (tenant, source,
    -- user_login, day) before the merge fires. Keep the latest extraction.
    SELECT *
    FROM {{ source('bronze_github_copilot', 'copilot_user_metrics') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY tenant_id, source_id, user_login, day
),
seats AS (
    -- Same dedup pattern on the bridge table; we only need login → email.
    SELECT
        tenant_id,
        source_id,
        user_login,
        lower(trim(user_email)) AS user_email
    FROM (
        SELECT *
        FROM {{ source('bronze_github_copilot', 'copilot_seats') }}
        ORDER BY _airbyte_extracted_at DESC
        LIMIT 1 BY tenant_id, source_id, user_login
    )
    WHERE user_email IS NOT NULL AND trim(user_email) != ''
)

SELECT
    m.tenant_id                                                            AS insight_tenant_id,
    m.source_id,
    CAST(concat(
        coalesce(m.tenant_id, ''), '-',
        coalesce(m.source_id, ''), '-',
        coalesce(m.user_login, ''), '-',
        coalesce(m.day, '')
    ) AS String)                                                           AS unique_key,
    s.user_email                                                           AS email,
    -- Copilot activity is attributed by user (login → email), not API key.
    CAST(NULL AS Nullable(String))                                         AS api_key_id,
    toDate(parseDateTimeBestEffortOrNull(m.day))                           AS day,
    'copilot'                                                              AS tool,
    -- session_count: Copilot doesn't expose a per-day session counter;
    -- presence of an activity row implies at least one active session.
    -- Match Cursor's convention: 1 per active day.
    toUInt32(1)                                                            AS session_count,
    toUInt32(coalesce(m.loc_added_sum, 0))                                 AS lines_added,
    toUInt32(coalesce(m.loc_deleted_sum, 0))                               AS lines_removed,
    -- See header comment — Copilot reports AI-accepted lines only.
    CAST(NULL AS Nullable(UInt32))                                         AS total_lines_added,
    CAST(NULL AS Nullable(UInt32))                                         AS total_lines_removed,
    -- code_generation_activity_count = number of code generation events
    -- (a proxy for "suggestions offered"). The API does not split offered
    -- from rejected, so this is the closest mappable signal.
    toUInt32(coalesce(m.code_generation_activity_count, 0))                AS tool_use_offered,
    toUInt32(coalesce(m.code_acceptance_activity_count, 0))                AS tool_use_accepted,
    -- #262: `completions_count` dropped from class_ai_dev_usage — it was numerically identical
    -- to tool_use_accepted (code_acceptance_activity_count). Same drop applied to cursor and
    -- claude_enterprise in PR #262; copilot now aligned.
    -- Boolean → activity-marker mapping: 1 marks a day where the user
    -- engaged with the surface; downstream uses these as flags, not
    -- absolute counters.
    if(coalesce(m.used_agent, false), toUInt32(1), CAST(NULL AS Nullable(UInt32)))   AS agent_sessions,
    if(coalesce(m.used_chat,  false), toUInt32(1), CAST(NULL AS Nullable(UInt32)))   AS chat_requests,
    -- Per-seat subscription, no per-event cost.
    CAST(NULL AS Nullable(UInt32))                                         AS cost_cents,
    -- Per-user attribution requires the org-level pull_requests breakdown
    -- (copilot_org_metrics.pull_requests) — not available at user grain
    -- without joining to a separate identity source. NULL for now.
    CAST(NULL AS Nullable(UInt32))                                         AS commits_count,
    CAST(NULL AS Nullable(UInt32))                                         AS pull_requests_count,
    -- prs_with_cc_count / prs_total_count: Claude Team-only (Anthropic GitHub-app attribution).
    -- Copilot exposes org-level PR metrics in copilot_org_metrics.pull_requests but not at
    -- user grain without a separate identity join. Structural NULL — column required for
    -- UNION ALL parity with claude_team__ai_dev_usage.
    CAST(NULL AS Nullable(UInt32))                                         AS prs_with_cc_count,
    CAST(NULL AS Nullable(UInt32))                                         AS prs_total_count,
    -- Activity-flag breakdown packed as JSON. Captures `used_cli` (no
    -- column slot) and re-surfaces the chat/agent flags for downstream
    -- consumers that want raw boolean state rather than the marker
    -- counts above.
    concat(
        '{"used_chat":',  if(coalesce(m.used_chat,  false), 'true', 'false'),
        ',"used_agent":', if(coalesce(m.used_agent, false), 'true', 'false'),
        ',"used_cli":',   if(coalesce(m.used_cli,   false), 'true', 'false'),
        '}'
    )                                                                      AS tool_action_breakdown_json,
    'copilot'                                                              AS source,
    'insight_github_copilot'                                               AS data_source,
    parseDateTime64BestEffortOrNull(coalesce(m.collected_at, ''), 3)       AS collected_at,
    toUnixTimestamp64Milli(m._airbyte_extracted_at)                        AS _version
FROM metrics m
LEFT JOIN seats s
    ON  m.tenant_id = s.tenant_id
    AND m.source_id = s.source_id
    AND m.user_login = s.user_login
WHERE s.user_email IS NOT NULL
  AND m.day IS NOT NULL
  AND m.day != ''
  AND (
       coalesce(m.code_acceptance_activity_count, 0) > 0
    OR coalesce(m.code_generation_activity_count, 0) > 0
    OR coalesce(m.loc_added_sum, 0) > 0
    OR coalesce(m.user_initiated_interaction_count, 0) > 0
    OR coalesce(m.used_chat, false)
    OR coalesce(m.used_agent, false)
    OR coalesce(m.used_cli, false)
  )
{% if is_incremental() %}
  -- 7-day re-process window: must be >= the connector's metrics_lookback_days
  -- (default 7, source_github_copilot) so days the connector re-fetches when a
  -- Copilot report lands late or is restated actually reach Silver. A narrower
  -- window would let those days sit in Bronze but be dropped here (older than
  -- max(day) - N), wasting the connector lookback. (Refs #1354.)
  AND toDate(parseDateTimeBestEffortOrNull(m.day)) > (
      SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 7 DAY
      FROM {{ this }}
  )
{% endif %}
