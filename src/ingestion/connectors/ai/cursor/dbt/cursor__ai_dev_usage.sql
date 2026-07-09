-- depends_on: {{ ref('cursor__bronze_promoted') }}
-- Bronze → Silver step 1: Cursor per-user per-day usage → class_ai_dev_usage
--
-- Source: bronze_cursor.cursor_daily_usage — daily aggregate stream from
-- POST /teams/daily-usage-data. One row per (userId, date) when the user was
-- active that day (isActive=true).
--
-- Filter: isActive=true AND email IS NOT NULL AND trim(email)!=''.
--
-- session_count semantics: Cursor does not expose a per-day "session count" —
-- it exposes chat/composer/agent request counters instead. For class_ai_dev_usage
-- we set session_count=1 per active day, which matches the concept of
-- "a day the user used the tool". Alternative counters (chatRequests,
-- agentRequests, totalTabsAccepted) are carried through as dedicated columns.
--
-- Note: the previous implementation of this model read cursor_usage_events* and
-- produced per-event rows with token granularity. It was replaced with a per-day
-- aggregation because class_ai_dev_usage consumers expect (tenant, email, day,
-- tool) granularity. Per-event / token-level data remains in Bronze and can be
-- tapped by a separate staging model if ever needed.
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['cursor', 'silver:class_ai_dev_usage']
) }}

SELECT
    tenant_id                                       AS insight_tenant_id,
    source_id,
    CAST(concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', coalesce(userId, ''), '-', toString(toDate(fromUnixTimestamp64Milli(CAST(date AS Int64))))) AS String)
                                                    AS unique_key,
    lower(trim(email))                              AS email,
    CAST(NULL AS Nullable(String))                  AS api_key_id,
    toDate(fromUnixTimestamp64Milli(CAST(date AS Int64))) AS day,
    'cursor'                                        AS tool,
    'Cursor'                                        AS tool_label,
    toUInt32(1)                                     AS session_count,
    CAST(NULL AS Nullable(UInt32))                  AS conversation_count,
    toUInt32(coalesce(acceptedLinesAdded, 0))       AS lines_added,
    toUInt32(coalesce(acceptedLinesDeleted, 0))     AS lines_removed,
    -- total_lines_added/removed = ALL lines the user wrote/deleted that day
    -- (not just AI-accepted ones). Needed by gold metrics like
    -- ai_loc_share = accepted/total to express AI contribution percentage.
    toUInt32(coalesce(totalLinesAdded, 0))          AS total_lines_added,
    toUInt32(coalesce(totalLinesDeleted, 0))        AS total_lines_removed,
    toUInt32OrNull(toString(totalTabsShown))        AS tool_use_offered,
    toUInt32OrNull(toString(totalTabsAccepted))     AS tool_use_accepted,
    -- #262: `completions_count` was numerically identical to tool_use_accepted
    -- (both = totalTabsAccepted) and dropped from class_ai_dev_usage.
    toUInt32OrNull(toString(agentRequests))         AS agent_sessions,
    toUInt32(coalesce(chatRequests, 0) + coalesce(composerRequests, 0))
                                                    AS chat_requests,
    CAST(NULL AS Nullable(UInt32))                  AS cost_cents,
    -- CE-specific columns — NULL for Cursor (Cursor does not expose git-level attribution)
    CAST(NULL AS Nullable(UInt32))                  AS commits_count,
    CAST(NULL AS Nullable(UInt32))                  AS pull_requests_count,
    -- prs_with_cc_count / prs_total_count: Claude Team-only (Anthropic GitHub-app attribution).
    -- Cursor does not expose PR-level attribution at user grain.
    -- Structural NULL per Silver NULL-policy (presence of column required for UNION ALL parity).
    CAST(NULL AS Nullable(UInt32))                  AS prs_with_cc_count,
    CAST(NULL AS Nullable(UInt32))                  AS prs_total_count,
    CAST(NULL AS Nullable(String))                  AS tool_action_breakdown_json,
    'cursor'                                        AS source,
    'insight_cursor'                                AS data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version
FROM {{ source('bronze_cursor', 'cursor_daily_usage') }}
WHERE isActive = true
  AND email IS NOT NULL
  AND trim(email) != ''
  -- Defensive: bronze can occasionally carry NULL `date`. Without this guard
  -- CAST(NULL AS Int64) → 0 and fromUnixTimestamp64Milli(0) → 1970-01-01,
  -- which silently corrupts the incremental boundary (max(day) gets stuck
  -- at the epoch) and emits a phantom 1970 row into Silver.
  AND date IS NOT NULL
{% if is_incremental() %}
  -- Empty-table guard. On a freshly-truncated `this` (the e2e rig resets staging
  -- between tests) `max(day)` is the Date epoch (1970-01-01), and `- INTERVAL 3 DAY`
  -- underflows the Date range and wraps to ~2149-06-04 — which would filter out
  -- every row and leave the model empty. Short-circuit when the table is empty so
  -- the full set is (re)loaded. Mirrors the m365__collab_* watermark guard.
  AND (
    (SELECT count() FROM {{ this }}) = 0
    OR toDate(fromUnixTimestamp64Milli(CAST(date AS Int64))) > (
        SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 3 DAY
        FROM {{ this }}
    )
  )
{% endif %}
