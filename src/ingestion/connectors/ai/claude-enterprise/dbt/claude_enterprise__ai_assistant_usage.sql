-- depends_on: {{ ref('claude_enterprise__bronze_promoted') }}
-- Bronze → Silver: Claude Enterprise per-user per-day assistant surface usage.
--
-- Source: bronze_claude_enterprise.claude_enterprise_users — per-user daily metrics.
-- Each Bronze row can produce up to 5 staging rows (one per non-empty surface).
--
-- Surface mapping:
--   surface='chat'       ← chat_* fields (conversations, messages, projects, files, skills, etc.)
--   surface='excel'      ← excel_* fields
--   surface='powerpoint' ← powerpoint_* fields
--   surface='cowork'     ← cowork_* fields (sessions, messages, actions, skills, dispatch turns)
--   surface='cross'      ← web_search_count (cross-surface counter, not attributable to a single product)
--
-- All rows carry tool='claude' (vendor discriminator for class_ai_assistant_usage).
-- Future: tool='chatgpt' for OpenAI Compliance API, tool='gemini' for Google.
--
-- Previously this data fed class_ai_api_usage via the now-deprecated
-- claude_enterprise__ai_api_usage model. Enterprise engagement data represents
-- human ↔ Claude assistant interaction (not programmatic API consumption), so it
-- belongs in class_ai_assistant_usage instead.

{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['claude-enterprise', 'silver:class_ai_assistant_usage']
) }}

WITH base AS (
    SELECT
        tenant_id,
        source_id,
        toDate(date)                                AS day,
        lower(trim(user_email))                     AS email,
        chat_conversation_count,
        chat_message_count,
        chat_files_uploaded_count,
        chat_artifacts_created_count,
        chat_projects_created_count,
        chat_projects_used_count,
        chat_skills_used_count,
        chat_connectors_used_count,
        chat_thinking_message_count,
        web_search_count,
        chat_metrics_json,
        excel_session_count,
        excel_message_count,
        powerpoint_session_count,
        powerpoint_message_count,
        office_metrics_json,
        cowork_session_count,
        cowork_message_count,
        cowork_action_count,
        cowork_dispatch_turn_count,
        cowork_skills_used_count,
        cowork_metrics_json,
        CAST(parseDateTime64BestEffortOrNull(coalesce(collected_at, ''), 3) AS Nullable(DateTime64(3))) AS collected_at,
        toUnixTimestamp64Milli(_airbyte_extracted_at) AS _version
    FROM (
        -- Bronze deduplication: see claude_enterprise__ai_dev_usage.sql for rationale.
        -- Once promote_bronze_to_rmt is enabled (ADR-0002), this LIMIT 1 BY becomes
        -- a no-op but stays as defensive depth.
        SELECT *
        FROM {{ source('bronze_claude_enterprise', 'claude_enterprise_users') }}
        ORDER BY _airbyte_extracted_at DESC
        LIMIT 1 BY tenant_id, source_id, user_id, date
    )
    WHERE user_email IS NOT NULL
      AND trim(user_email) != ''
    {% if is_incremental() %}
      AND toDate(date) > (
          SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 3 DAY
          FROM {{ this }}
      )
    {% endif %}
),

-- ── surface = 'chat' ───────────────────────────────────────────────────────
chat AS (
    SELECT
        tenant_id                                                       AS insight_tenant_id,
        source_id,
        CAST(concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', coalesce(email, ''), '-', toString(day), '-chat') AS String)
                                                                        AS unique_key,
        email,
        day,
        'claude'                                                        AS tool,
        'chat'                                                          AS surface,
        -- chat is not session-bounded in the API → session_count NULL
        CAST(NULL AS Nullable(UInt32))                                  AS session_count,
        toUInt32OrNull(toString(chat_conversation_count))               AS conversation_count,
        toUInt32OrNull(toString(chat_message_count))                    AS message_count,
        -- chat has no per-action counter (skills/connectors/thinking are tracked separately)
        CAST(NULL AS Nullable(UInt32))                                  AS action_count,
        toUInt32OrNull(toString(chat_files_uploaded_count))             AS files_uploaded_count,
        toUInt32OrNull(toString(chat_artifacts_created_count))          AS artifacts_created_count,
        toUInt32OrNull(toString(chat_projects_created_count))           AS projects_created_count,
        toUInt32OrNull(toString(chat_projects_used_count))              AS projects_used_count,
        toUInt32OrNull(toString(chat_skills_used_count))                AS skills_used_count,
        toUInt32OrNull(toString(chat_connectors_used_count))            AS connectors_used_count,
        toUInt32OrNull(toString(chat_thinking_message_count))           AS thinking_message_count,
        CAST(NULL AS Nullable(UInt32))                                  AS dispatch_turn_count,
        -- web_search emitted on its own surface='cross' row, not here
        CAST(NULL AS Nullable(UInt32))                                  AS search_count,
        CAST(NULL AS Nullable(UInt32))                                  AS cost_cents,
        chat_metrics_json                                               AS surface_metrics_json,
        'claude_enterprise'                                             AS source,
        'insight_claude_enterprise'                                     AS data_source,
        collected_at,
        _version
    FROM base
    -- Emit a chat row whenever ANY chat-surface counter is non-zero. Anthropic
    -- can report file uploads / skills / thinking turns on days with no logged
    -- conversation or message; a narrower filter would silently drop those rows.
    WHERE coalesce(chat_conversation_count, 0) > 0
       OR coalesce(chat_message_count, 0) > 0
       OR coalesce(chat_files_uploaded_count, 0) > 0
       OR coalesce(chat_artifacts_created_count, 0) > 0
       OR coalesce(chat_projects_created_count, 0) > 0
       OR coalesce(chat_projects_used_count, 0) > 0
       OR coalesce(chat_skills_used_count, 0) > 0
       OR coalesce(chat_connectors_used_count, 0) > 0
       OR coalesce(chat_thinking_message_count, 0) > 0
),

-- ── surface = 'excel' ──────────────────────────────────────────────────────
excel AS (
    SELECT
        tenant_id                                                       AS insight_tenant_id,
        source_id,
        CAST(concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', coalesce(email, ''), '-', toString(day), '-excel') AS String)
                                                                        AS unique_key,
        email,
        day,
        'claude'                                                        AS tool,
        'excel'                                                         AS surface,
        toUInt32OrNull(toString(excel_session_count))                   AS session_count,
        CAST(NULL AS Nullable(UInt32))                                  AS conversation_count,
        toUInt32OrNull(toString(excel_message_count))                   AS message_count,
        CAST(NULL AS Nullable(UInt32))                                  AS action_count,
        CAST(NULL AS Nullable(UInt32))                                  AS files_uploaded_count,
        CAST(NULL AS Nullable(UInt32))                                  AS artifacts_created_count,
        CAST(NULL AS Nullable(UInt32))                                  AS projects_created_count,
        CAST(NULL AS Nullable(UInt32))                                  AS projects_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS skills_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS connectors_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS thinking_message_count,
        CAST(NULL AS Nullable(UInt32))                                  AS dispatch_turn_count,
        CAST(NULL AS Nullable(UInt32))                                  AS search_count,
        CAST(NULL AS Nullable(UInt32))                                  AS cost_cents,
        office_metrics_json                                             AS surface_metrics_json,
        'claude_enterprise'                                             AS source,
        'insight_claude_enterprise'                                     AS data_source,
        collected_at,
        _version
    FROM base
    WHERE coalesce(excel_session_count, 0) > 0
       OR coalesce(excel_message_count, 0) > 0
),

-- ── surface = 'powerpoint' ─────────────────────────────────────────────────
powerpoint AS (
    SELECT
        tenant_id                                                       AS insight_tenant_id,
        source_id,
        CAST(concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', coalesce(email, ''), '-', toString(day), '-powerpoint') AS String)
                                                                        AS unique_key,
        email,
        day,
        'claude'                                                        AS tool,
        'powerpoint'                                                    AS surface,
        toUInt32OrNull(toString(powerpoint_session_count))              AS session_count,
        CAST(NULL AS Nullable(UInt32))                                  AS conversation_count,
        toUInt32OrNull(toString(powerpoint_message_count))              AS message_count,
        CAST(NULL AS Nullable(UInt32))                                  AS action_count,
        CAST(NULL AS Nullable(UInt32))                                  AS files_uploaded_count,
        CAST(NULL AS Nullable(UInt32))                                  AS artifacts_created_count,
        CAST(NULL AS Nullable(UInt32))                                  AS projects_created_count,
        CAST(NULL AS Nullable(UInt32))                                  AS projects_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS skills_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS connectors_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS thinking_message_count,
        CAST(NULL AS Nullable(UInt32))                                  AS dispatch_turn_count,
        CAST(NULL AS Nullable(UInt32))                                  AS search_count,
        CAST(NULL AS Nullable(UInt32))                                  AS cost_cents,
        office_metrics_json                                             AS surface_metrics_json,
        'claude_enterprise'                                             AS source,
        'insight_claude_enterprise'                                     AS data_source,
        collected_at,
        _version
    FROM base
    WHERE coalesce(powerpoint_session_count, 0) > 0
       OR coalesce(powerpoint_message_count, 0) > 0
),

-- ── surface = 'cowork' ─────────────────────────────────────────────────────
cowork AS (
    SELECT
        tenant_id                                                       AS insight_tenant_id,
        source_id,
        CAST(concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', coalesce(email, ''), '-', toString(day), '-cowork') AS String)
                                                                        AS unique_key,
        email,
        day,
        'claude'                                                        AS tool,
        'cowork'                                                        AS surface,
        toUInt32OrNull(toString(cowork_session_count))                  AS session_count,
        CAST(NULL AS Nullable(UInt32))                                  AS conversation_count,
        toUInt32OrNull(toString(cowork_message_count))                  AS message_count,
        toUInt32OrNull(toString(cowork_action_count))                   AS action_count,
        CAST(NULL AS Nullable(UInt32))                                  AS files_uploaded_count,
        CAST(NULL AS Nullable(UInt32))                                  AS artifacts_created_count,
        CAST(NULL AS Nullable(UInt32))                                  AS projects_created_count,
        CAST(NULL AS Nullable(UInt32))                                  AS projects_used_count,
        toUInt32OrNull(toString(cowork_skills_used_count))              AS skills_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS connectors_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS thinking_message_count,
        toUInt32OrNull(toString(cowork_dispatch_turn_count))            AS dispatch_turn_count,
        CAST(NULL AS Nullable(UInt32))                                  AS search_count,
        CAST(NULL AS Nullable(UInt32))                                  AS cost_cents,
        cowork_metrics_json                                             AS surface_metrics_json,
        'claude_enterprise'                                             AS source,
        'insight_claude_enterprise'                                     AS data_source,
        collected_at,
        _version
    FROM base
    -- Same broad filter as chat: emit row whenever any cowork counter signals
    -- activity. Action-only / dispatch-only days are observed in real data.
    WHERE coalesce(cowork_session_count, 0) > 0
       OR coalesce(cowork_message_count, 0) > 0
       OR coalesce(cowork_action_count, 0) > 0
       OR coalesce(cowork_dispatch_turn_count, 0) > 0
       OR coalesce(cowork_skills_used_count, 0) > 0
),

-- ── surface = 'cross' (web_search) ─────────────────────────────────────────
-- Per gist Q2: web_search is cross-surface (not attributable to chat / excel /
-- powerpoint / cowork specifically). Emitted as its own row with search_count
-- populated and all other counters NULL — keeps the per-surface schema clean.
cross_surface AS (
    SELECT
        tenant_id                                                       AS insight_tenant_id,
        source_id,
        CAST(concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', coalesce(email, ''), '-', toString(day), '-cross') AS String)
                                                                        AS unique_key,
        email,
        day,
        'claude'                                                        AS tool,
        'cross'                                                         AS surface,
        CAST(NULL AS Nullable(UInt32))                                  AS session_count,
        CAST(NULL AS Nullable(UInt32))                                  AS conversation_count,
        CAST(NULL AS Nullable(UInt32))                                  AS message_count,
        CAST(NULL AS Nullable(UInt32))                                  AS action_count,
        CAST(NULL AS Nullable(UInt32))                                  AS files_uploaded_count,
        CAST(NULL AS Nullable(UInt32))                                  AS artifacts_created_count,
        CAST(NULL AS Nullable(UInt32))                                  AS projects_created_count,
        CAST(NULL AS Nullable(UInt32))                                  AS projects_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS skills_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS connectors_used_count,
        CAST(NULL AS Nullable(UInt32))                                  AS thinking_message_count,
        CAST(NULL AS Nullable(UInt32))                                  AS dispatch_turn_count,
        toUInt32OrNull(toString(web_search_count))                      AS search_count,
        CAST(NULL AS Nullable(UInt32))                                  AS cost_cents,
        CAST(NULL AS Nullable(String))                                  AS surface_metrics_json,
        'claude_enterprise'                                             AS source,
        'insight_claude_enterprise'                                     AS data_source,
        collected_at,
        _version
    FROM base
    WHERE coalesce(web_search_count, 0) > 0
)

SELECT * FROM chat
UNION ALL
SELECT * FROM excel
UNION ALL
SELECT * FROM powerpoint
UNION ALL
SELECT * FROM cowork
UNION ALL
SELECT * FROM cross_surface
