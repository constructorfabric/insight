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
-- Cost: Cursor prices usage per event, so cost_cents comes from
-- cursor__usage_events (the deduplicated per-event surface) aggregated to
-- email × day. See schema.yml for what the amount means and why `chargedCents`
-- is summed alone.
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

WITH daily AS (
    SELECT
        *,
        toDate(fromUnixTimestamp64Milli(CAST(date AS Int64))) AS day_key,
        row_number() OVER (
            PARTITION BY coalesce(tenant_id, ''), coalesce(source_id, ''),
                         lower(trim(email)), toDate(fromUnixTimestamp64Milli(CAST(date AS Int64)))
            ORDER BY coalesce(userId, '')
        )                                                 AS email_day_rank
    FROM (
        -- Read-time dedup of the Bronze RMT (ADR-0001): the daily-usage stream
        -- re-fetches the cursor-boundary day, so Bronze holds one row per
        -- (person-day × sync) until a background merge collapses them. Reading
        -- them all would emit several Silver rows per unique_key whose _version
        -- can tie once the per-event stamp dominates — a tie makes the
        -- ReplacingMergeTree winner, and therefore the activity counters,
        -- arbitrary.
        SELECT *
        FROM {{ source('bronze_cursor', 'cursor_daily_usage') }}
        ORDER BY _airbyte_extracted_at DESC
        LIMIT 1 BY unique_key
    )
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
)

SELECT
    d.tenant_id                                     AS insight_tenant_id,
    d.source_id                                     AS source_id,
    CAST(concat(coalesce(d.tenant_id, ''), '-', coalesce(d.source_id, ''), '-', coalesce(d.userId, ''), '-', toString(d.day_key)) AS String)
                                                    AS unique_key,
    lower(trim(d.email))                            AS email,
    CAST(NULL AS Nullable(String))                  AS api_key_id,
    d.day_key                                       AS day,
    'cursor'                                        AS tool,
    toUInt32(1)                                     AS session_count,
    CAST(NULL AS Nullable(UInt32))                  AS conversation_count,
    toUInt32(coalesce(d.acceptedLinesAdded, 0))     AS lines_added,
    toNullable(toUInt32(coalesce(d.acceptedLinesDeleted, 0)))   AS lines_removed,
    -- total_lines_added/removed = ALL lines the user wrote/deleted that day
    -- (not just AI-accepted ones). Needed by gold metrics like
    -- ai_loc_share = accepted/total to express AI contribution percentage.
    toNullable(toUInt32(coalesce(d.totalLinesAdded, 0)))        AS total_lines_added,
    toNullable(toUInt32(coalesce(d.totalLinesDeleted, 0)))      AS total_lines_removed,
    toUInt32OrNull(toString(d.totalTabsShown))      AS tool_use_offered,
    toUInt32OrNull(toString(d.totalTabsAccepted))   AS tool_use_accepted,
    -- #262: `completions_count` was numerically identical to tool_use_accepted
    -- (both = totalTabsAccepted) and dropped from class_ai_dev_usage.
    toUInt32OrNull(toString(d.agentRequests))       AS agent_sessions,
    toNullable(toUInt32(coalesce(d.chatRequests, 0) + coalesce(d.composerRequests, 0)))
                                                    AS chat_requests,
    -- Rank guard: cost is aggregated per (tenant, source, email, day) because
    -- Cursor's events carry only userEmail, while a row is identified by userId.
    -- Two userIds sharing an email on one day would otherwise each take the
    -- full day's cost and double it downstream.
    if(coalesce(c.has_priced_event, 0) = 1 AND d.email_day_rank = 1,
       toNullable(toUInt32(greatest(0, c.charged_cents))),
       CAST(NULL AS Nullable(UInt32)))              AS cost_cents,
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
    CAST(d._airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    -- coalesce keeps _version non-nullable: an unmatched LEFT JOIN yields NULL
    -- under join_use_nulls=1, and ReplacingMergeTree rejects a Nullable version.
    greatest(toUnixTimestamp64Milli(d._airbyte_extracted_at), coalesce(c.cost_version, toInt64(0)))
                                                    AS _version,
    -- Cursor reports an is-active flag per day, not a seat lifecycle state.
    CAST(NULL AS Nullable(String))                  AS seat_status
FROM daily AS d
LEFT JOIN {{ ref('cursor__event_cost_daily') }} AS c
       ON coalesce(d.tenant_id, '') = c.tenant_key
      AND coalesce(d.source_id, '') = c.source_key
      AND lower(trim(d.email)) = c.email
      AND d.day_key = c.day
