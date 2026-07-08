-- depends_on: {{ ref('claude_team__bronze_promoted') }}
-- Bronze → Silver step 1: Claude Team per-user per-day CC usage → class_ai_dev_usage
--
-- Source: bronze_claude_team.claude_team_code_metrics — daily aggregate stream
-- pulled via the customer-deployed claude-team-proxy from the claude.ai
-- web API (/api/organizations/{org_id}/claude_code/metrics). One row per
-- (email, metric_date) — metrics already aggregated to daily grain by the API.
--
-- Filters:
--   status = 'active'             — drop deactivated seats (per PR #553)
--   email IS NOT NULL / != ''     — rows without email cannot be attributed
--   metric_date IS NOT NULL       — guard against phantom 1970-01-01 rows
--
-- session_count: `total_sessions`. DQ note: 5/34 sample users had
--   sessions=0 while lines_accepted>0 — headless / `cc -p` invocations are
--   excluded from the session counter by Anthropic. Not a model bug.
--
-- lines_added: `total_lines_accepted` — AI-accepted lines. Same semantics as
--   Enterprise `code_lines_added`. Total keystrokes not available → NULL.
--
-- cost_cents: `total_cost` (decimal-as-string, e.g. "1.23") cast to cents.
--   Claude Team is the first per-user-per-day cost source in Silver; all
--   other sources expose cost at org/workspace grain only.
--
-- prs_with_cc_count / prs_total_count: Anthropic GitHub-app attribution.
--   Populated only on tenants with the app connected; zero on orgs without it.
--   ⚠️ prs_total_count may be a period-aggregate (cumulative), not daily —
--   verify against a tenant with a connected GitHub app.
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['claude-team', 'silver:class_ai_dev_usage']
) }}

SELECT
    tenant_id                                           AS insight_tenant_id,
    source_id,
    -- Unique key: tenant-source-email-day (mirrors claude_admin pattern)
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(source_id, ''), '-',
        lower(trim(coalesce(email, ''))), '-',
        coalesce(metric_date, '')
    ) AS String)                                        AS unique_key,
    lower(trim(email))                                  AS email,
    -- Session-based auth (operator sessionKey cookie); users identified by
    -- email, not API keys.
    CAST(NULL AS Nullable(String))                      AS api_key_id,
    toDate(metric_date)                                 AS day,
    'claude_code'                                       AS tool,
    'Claude Code'                                       AS tool_label,
    toUInt32(coalesce(total_sessions, 0))               AS session_count,
    toUInt32OrNull(toString(total_sessions))            AS conversation_count,
    toUInt32(coalesce(total_lines_accepted, 0))         AS lines_added,
    -- NULL per NULL-policy (PR #553): Claude Team does not expose AI-removed
    -- lines — structural absence, not zero.
    CAST(NULL AS Nullable(UInt32))                      AS lines_removed,
    -- Total keystrokes (AI + manual) not available from the web API.
    CAST(NULL AS Nullable(UInt32))                      AS total_lines_added,
    CAST(NULL AS Nullable(UInt32))                      AS total_lines_removed,
    -- Inline-completion offered/accepted/rejected counters not surfaced by
    -- the Team plan API — structural NULL, not zero.
    CAST(NULL AS Nullable(UInt32))                      AS tool_use_offered,
    CAST(NULL AS Nullable(UInt32))                      AS tool_use_accepted,
    CAST(NULL AS Nullable(UInt32))                      AS agent_sessions,
    CAST(NULL AS Nullable(UInt32))                      AS chat_requests,
    -- total_cost is a decimal-as-string (e.g. "1.230000"). Convert to cents.
    -- NULL-safe: returns NULL when total_cost IS NULL or not parseable.
    toUInt32OrNull(toString(round(toFloat64OrNull(total_cost) * 100)))
                                                        AS cost_cents,
    -- Git-level attribution: commits not exposed by the Team plan API.
    CAST(NULL AS Nullable(UInt32))                      AS commits_count,
    -- pull_requests_count = Enterprise-specific (code_pull_request_count).
    -- Claude Team PR counts go into the dedicated prs_total_count column.
    CAST(NULL AS Nullable(UInt32))                      AS pull_requests_count,
    -- New Silver columns for Claude Team PR attribution (PR #553):
    toUInt32OrNull(toString(prs_with_cc))               AS prs_with_cc_count,
    toUInt32OrNull(toString(total_prs))                 AS prs_total_count,
    CAST(NULL AS Nullable(String))                      AS tool_action_breakdown_json,
    -- source='claude_team': connector identifier per the coverage matrix
    -- (PR #553). Transport is Playwright-based but the discriminator follows
    -- the connector name, not the transport.
    'claude_team'                                       AS source,
    data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version
FROM {{ source('bronze_claude_team', 'claude_team_code_metrics') }}
WHERE status = 'active'
  AND email IS NOT NULL
  AND trim(email) != ''
  -- Guard against NULL metric_date: toDate(NULL) → 1970-01-01 silently
  -- corrupts the incremental boundary (same guard as cursor__ai_dev_usage).
  AND metric_date IS NOT NULL
{% if is_incremental() %}
  -- Empty-table guard. Over an empty `this` (the e2e rig resets staging between
  -- tests) `max(day)` is the Date epoch (1970-01-01) and `- INTERVAL 3 DAY`
  -- underflows the Date range, wrapping to ~2149-06-04 — which filters out every
  -- row and leaves the model empty. Short-circuit when empty so the full set is
  -- (re)loaded. Mirrors the cursor__ai_dev_usage / m365__collab_* guard.
  AND (
    (SELECT count() FROM {{ this }}) = 0
    OR toDate(metric_date) > (
        SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 3 DAY
        FROM {{ this }}
    )
  )
{% endif %}
