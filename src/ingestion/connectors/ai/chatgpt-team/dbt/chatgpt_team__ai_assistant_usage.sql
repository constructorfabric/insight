-- depends_on: {{ ref('chatgpt_team__bronze_promoted') }}
-- Bronze → Silver: ChatGPT Team per-user per-day chat usage → class_ai_assistant_usage.
--
-- Source: bronze_chatgpt_team.chatgpt_team_chat_activity — daily per-user chat
-- counts pulled via the chatgpt-team-proxy from chatgpt.com's
-- /backend-api/accounts/{id}/analytics/user_list. One row per (email, date).
--
-- ChatGPT chat is a single surface → one staging row per (email, day) with
-- surface='chat', tool='chatgpt' (vendor discriminator; cf. 'claude').
--
-- Column contract MUST match the other class_ai_assistant_usage sources
-- (claude_enterprise) — union_by_tag does positional UNION ALL. Keep column
-- order/names/types identical to claude_enterprise__ai_assistant_usage.
--
-- Mapping notes:
--   message_count ← messages          — total messages that day.
--   conversation_count ← NULL         — user_list reports messages, not conversations.
--   The per-bucket message splits (gpt/tool/connector/project), credits_used and
--   seat_type are preserved in surface_metrics_json (the shared schema has no
--   dedicated columns for them).
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['chatgpt-team', 'silver:class_ai_assistant_usage']
) }}

SELECT
    tenant_id                                                       AS insight_tenant_id,
    source_id,
    CAST(concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', lower(trim(coalesce(email, ''))), '-', coalesce(date, ''), '-chat') AS String)
                                                                    AS unique_key,
    lower(trim(email))                                              AS email,
    toDate(date)                                                    AS day,
    'chatgpt'                                                       AS tool,
    'chat'                                                          AS surface,
    -- chat is not session-bounded in the API → session_count NULL
    CAST(NULL AS Nullable(UInt32))                                  AS session_count,
    CAST(NULL AS Nullable(UInt32))                                  AS conversation_count,
    toUInt32OrNull(toString(messages))                              AS message_count,
    CAST(NULL AS Nullable(UInt32))                                  AS action_count,
    CAST(NULL AS Nullable(UInt32))                                  AS files_uploaded_count,
    CAST(NULL AS Nullable(UInt32))                                  AS artifacts_created_count,
    CAST(NULL AS Nullable(UInt32))                                  AS projects_created_count,
    -- project_messages is a message count, not a distinct-projects count → JSON
    CAST(NULL AS Nullable(UInt32))                                  AS projects_used_count,
    CAST(NULL AS Nullable(UInt32))                                  AS skills_used_count,
    -- connector_messages is a message count, not a distinct-connectors count → JSON
    CAST(NULL AS Nullable(UInt32))                                  AS connectors_used_count,
    CAST(NULL AS Nullable(UInt32))                                  AS thinking_message_count,
    CAST(NULL AS Nullable(UInt32))                                  AS dispatch_turn_count,
    CAST(NULL AS Nullable(UInt32))                                  AS search_count,
    CAST(NULL AS Nullable(UInt32))                                  AS cost_cents,
    CAST(toJSONString(map(
        'gpt_messages',       toString(coalesce(toUInt32OrNull(toString(gpt_messages)), 0)),
        'tool_messages',      toString(coalesce(toUInt32OrNull(toString(tool_messages)), 0)),
        'connector_messages', toString(coalesce(toUInt32OrNull(toString(connector_messages)), 0)),
        'project_messages',   toString(coalesce(toUInt32OrNull(toString(project_messages)), 0)),
        'credits_used',       toString(coalesce(credits_used, 0)),
        'seat_type',          coalesce(seat_type, '')
    )) AS Nullable(String))                                         AS surface_metrics_json,
    'chatgpt_team'                                                  AS source,
    data_source                                                     AS data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3)))          AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)                   AS _version
FROM (
    -- Bronze dedup: keep the latest extract per (email, date). Defensive depth —
    -- becomes a no-op once promote_bronze_to_rmt merges (ADR-0002), but guards
    -- against duplicate raw rows from multiple sync attempts.
    SELECT *
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_chat_activity') }}
    ORDER BY _airbyte_extracted_at DESC
    -- Dedup on the SAME normalized key the unique_key uses (lower(trim(email))),
    -- else two case-variant spellings collide on unique_key (unique-test fail).
    LIMIT 1 BY tenant_id, source_id, lower(trim(email)), date
)
WHERE email IS NOT NULL
  AND trim(email) != ''
  AND date IS NOT NULL
  -- Emit a row whenever any chat-surface counter signals activity.
  AND (
        coalesce(toUInt32OrNull(toString(messages)), 0) > 0
     OR coalesce(toUInt32OrNull(toString(tool_messages)), 0) > 0
     OR coalesce(toUInt32OrNull(toString(connector_messages)), 0) > 0
     OR coalesce(toUInt32OrNull(toString(project_messages)), 0) > 0
     OR coalesce(toUInt32OrNull(toString(gpt_messages)), 0) > 0
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
