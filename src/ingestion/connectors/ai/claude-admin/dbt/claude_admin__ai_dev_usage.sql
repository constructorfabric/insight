-- Bronze → Silver step 1: Claude Admin Claude Code usage → class_ai_dev_usage
--
-- Source: bronze_claude_admin.claude_admin_code_usage — daily Claude Code
-- activity from /v1/organizations/usage_report/claude_code.
--
-- Actor model:
--   • actor_type='user'      → actor_identifier = user email
--                              email column populated, api_key_id NULL
--   • actor_type='api_actor' → actor_identifier = API key *name*
--                              (e.g. "karsten-claude-code-key"). JOIN with
--                              claude_admin_api_keys on name to resolve api_key_id.
--                              email column NULL, api_key_id populated.
--                              Observed in real Virtuozzo data: 100% of activity
--                              is api_actor — org admins provision named per-
--                              developer keys rather than letting users auth
--                              directly. Silver Step 2 (Identity Resolution)
--                              maps api_key_id → person_id.
--
-- Note: Claude Code usage is also present in Enterprise data
-- (claude_enterprise_users.code_*). For orgs on the Enterprise subscription,
-- Enterprise is now the canonical feed for class_ai_dev_usage — see
-- claude_enterprise__ai_dev_usage.sql. Admin's claude_admin_code_usage is
-- retained here only to keep the staging table populated for downstream
-- joins / debugging; it is no longer tagged for silver:class_ai_dev_usage,
-- so it does not double-count via union_by_tag. Admin remains tagged for
-- silver:class_ai_api_usage (token-level metrics Enterprise does not publish).
--
-- Why prefer Enterprise here:
--   • user-grain attribution out of the box (user_email) — Admin reports
--     api_actor for 100% of activity in real-world data, requiring an extra
--     api_key_name → api_key_id → person_id resolution step.
--   • Enterprise also exposes adjacent surfaces (chat, cowork, office)
--     that Admin's code_usage endpoint does not.
--
-- Aggregation: Bronze has one row per (date, actor_identifier, terminal_type)
-- — we aggregate across terminal_type to match class_ai_dev_usage granularity
-- (tenant, email|api_key_id, day, tool).
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['claude-admin']
) }}

WITH usage_agg AS (
    SELECT
        tenant_id,
        insight_source_id                                   AS source_id,
        actor_type,
        lower(trim(actor_identifier))                       AS identifier_lc,
        toDate(date)                                        AS day,
        sum(coalesce(session_count, 0))                     AS sessions_sum,
        sum(coalesce(lines_added, 0))                       AS lines_added_sum,
        sum(coalesce(lines_removed, 0))                     AS lines_removed_sum,
        sum(coalesce(tool_use_accepted, 0))                 AS tool_accepted_sum,
        sum(coalesce(tool_use_rejected, 0))                 AS tool_rejected_sum,
        max(parseDateTime64BestEffortOrNull(coalesce(collected_at, ''), 3)) AS collected_at_max
    FROM {{ source('bronze_claude_admin', 'claude_admin_code_usage') }}
    WHERE actor_type IN ('user', 'api_actor')
      AND actor_identifier IS NOT NULL
      AND trim(actor_identifier) != ''
    {% if is_incremental() %}
      AND toDate(date) > (
          SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 3 DAY
          FROM {{ this }}
      )
    {% endif %}
    GROUP BY tenant_id, insight_source_id, actor_type, lower(trim(actor_identifier)), toDate(date)
),

-- api_key_name → api_key_id lookup (only names actually used in code_usage)
api_keys AS (
    SELECT
        tenant_id,
        id                                                  AS api_key_id,
        lower(trim(name))                                   AS name_lc
    FROM {{ source('bronze_claude_admin', 'claude_admin_api_keys') }}
    WHERE name IS NOT NULL AND trim(name) != ''
)

SELECT
    u.tenant_id                                             AS insight_tenant_id,
    u.source_id,
    -- Non-nullable unique_key: coalesce Nullable Bronze inputs to '' before
    -- concat and cast the result back to String. ClickHouse MergeTree ORDER BY
    -- rejects Nullable columns by default (allow_nullable_key=0).
    CAST(concat(
        coalesce(u.tenant_id, ''), '-',
        coalesce(u.source_id, ''), '-',
        u.actor_type, '-',
        u.identifier_lc, '-',
        toString(u.day)
    ) AS String)                                            AS unique_key,
    CASE WHEN u.actor_type = 'user' THEN u.identifier_lc END AS email,
    CASE WHEN u.actor_type = 'api_actor' THEN k.api_key_id END AS api_key_id,
    u.day,
    'claude_code'                                           AS tool,
    toUInt32(u.sessions_sum)                                AS session_count,
    toUInt32(u.lines_added_sum)                             AS lines_added,
    toUInt32(u.lines_removed_sum)                           AS lines_removed,
    -- total_lines_added/removed: Admin code_usage doesn't expose total keystrokes. NULL.
    CAST(NULL AS Nullable(UInt32))                          AS total_lines_added,
    CAST(NULL AS Nullable(UInt32))                          AS total_lines_removed,
    toUInt32(u.tool_accepted_sum + u.tool_rejected_sum)     AS tool_use_offered,
    toUInt32(u.tool_accepted_sum)                           AS tool_use_accepted,
    CAST(NULL AS Nullable(UInt32))                          AS completions_count,
    CAST(NULL AS Nullable(UInt32))                          AS agent_sessions,
    CAST(NULL AS Nullable(UInt32))                          AS chat_requests,
    CAST(NULL AS Nullable(UInt32))                          AS cost_cents,
    -- CE-specific columns — NULL for Admin (Admin code_usage does not expose git attribution).
    CAST(NULL AS Nullable(UInt32))                          AS commits_count,
    CAST(NULL AS Nullable(UInt32))                          AS pull_requests_count,
    CAST(NULL AS Nullable(String))                          AS tool_action_breakdown_json,
    'claude_admin'                                          AS source,
    'insight_claude_admin'                                  AS data_source,
    CAST(u.collected_at_max AS Nullable(DateTime64(3)))     AS collected_at,
    -- _version: aggregating model uses max(collected_at) as version proxy (epoch-ms).
    -- NULL collected_at falls back to 0 to keep _version non-nullable.
    coalesce(toUnixTimestamp64Milli(u.collected_at_max), toUInt64(0)) AS _version
FROM usage_agg u
LEFT JOIN api_keys k
    ON u.actor_type = 'api_actor'
   AND u.tenant_id = k.tenant_id
   AND u.identifier_lc = k.name_lc
-- Drop orphan api_actor rows where the name could not be resolved to an
-- api_key_id (deleted key, case/trim drift, delayed claude_admin_api_keys
-- ingest). Without this guard, such rows would emit both email=NULL and
-- api_key_id=NULL, polluting class_ai_dev_usage with activity that has no
-- attributable identity. user rows are never orphaned — actor_identifier
-- always carries the email.
WHERE u.actor_type = 'user'
   OR (u.actor_type = 'api_actor' AND k.api_key_id IS NOT NULL)
