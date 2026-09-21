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
--   conversation_count ← n_threads — the same thread count: a thread IS the
--                                   unit of conversation, and no separate
--                                   conversation counter exists upstream.
--   lines_added ← lines_added     — AI-accepted lines (from code_attribution.lines_of_code.added).
--   cost_cents ← NULL             — `credits` are Codex usage credits, not a currency amount.
--   Codex-only counters (credits, n_turns, text_tokens, current_streak) are
--   preserved in tool_action_breakdown_json so nothing is lost. They do not
--   admit a row: see the emission filter's invariant at the foot of the model.
--
-- Two endpoints feed Codex and each owns its fields. The leaderboard owns
-- email, name, lines_added and the thread/turn counters; the sessions
-- aggregate (chatgpt_team_codex_sessions_daily) owns the credit split and the
-- token split. They reconcile on (user_id, date); seen_in_sessions records the
-- outcome per row.
--
-- INVARIANT: the emission filter is unchanged by the sessions join. The
-- sessions counters are turns and NEW sessions, neither of which is a
-- contract column here — agent_sessions and chat_requests are Cursor's by the
-- class contract — so admitting a row on them would assert activity no class
-- column can evidence, which is the same thing the filter already refuses to
-- do for credits. Monetary usage and active_day stay separate concerns.
--
-- INVARIANT: `credits` is ON-DEMAND usage, not total Codex consumption — it
--   matches the vendor's own on-demand figure wherever one is published, and
--   Codex activity routinely records zero credits while still reporting tokens.
--   A person-day with credits = 0 is therefore ordinary, not a gap, which is
--   why the emission filter does not read it. Whether the uncredited part is
--   allowance-covered or unmetered is not established; either way it is
--   excluded from this number.
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

-- Completeness gate. The per-user endpoint pages over sort_by=credits, which
-- is not a total order — everyone who spent nothing ties on it — so a person
-- can sit past the page boundary in one request and before it in the next,
-- returned by neither, while every request succeeds. A read that lost somebody
-- must not become the day's state, so it is dropped whole and the day keeps
-- whatever the last sound read wrote.
WITH reference AS (

    -- The vendor's own headcount, as of the read that returned it. Keyed on
    -- the read and not the day: a day still in progress reports fewer people
    -- than it ends with, so judging an older read against a newer reference
    -- would reject sound reads.
    SELECT
        coalesce(tenant_id, '')                         AS insight_tenant_id,
        coalesce(source_id, '')                         AS source_id,
        toDateOrNull(date)                              AS day,
        JSONExtractInt(_airbyte_meta, 'sync_id')        AS read_id,
        -- Not aliased `total_users`: the alias shadows the source column and
        -- ClickHouse then resolves the outer reference to the aggregate itself.
        max(toInt64OrNull(toString(total_users)))       AS read_total_users
    -- FINAL: the envelope table is a ReplacingMergeTree, so an unmerged part
    -- can still hold a superseded headcount for the same read.
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_codex_user_daily_org') }} FINAL
    WHERE date IS NOT NULL
      AND trim(date) != ''
      AND total_users IS NOT NULL
    GROUP BY insight_tenant_id, source_id, day, read_id

),

first_reference AS (

    -- Which read first carried a reference on this instance. Reads before it
    -- predate the envelope stream and cannot be judged; that read and
    -- everything after must carry one, or they are rejected.
    --
    -- INVARIANT: the boundary is the READ, not its timestamp. Both streams are
    -- written by one job but not at one instant, so on the first sync after
    -- the upgrade the per-user rows can land a moment before the envelope that
    -- would judge them.
    SELECT
        insight_tenant_id,
        source_id,
        min(read_id)                                    AS since_read,
        toUInt8(1)                                      AS instance_has_references
    FROM reference
    GROUP BY insight_tenant_id, source_id

),

read_state AS (

    SELECT
        coalesce(tenant_id, '')                         AS insight_tenant_id,
        coalesce(source_id, '')                         AS source_id,
        toDateOrNull(date)                              AS day,
        JSONExtractInt(_airbyte_meta, 'sync_id')        AS read_id,
        -- Counts only rows the serving layer can attribute, so a headcount
        -- that includes someone arriving without an email fails the gate
        -- closed instead of passing on a body count.
        uniqExactIf(lower(trim(email)),
                    email IS NOT NULL AND trim(email) != '') AS people
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_codex_user_daily') }}
    WHERE date IS NOT NULL
      AND trim(date) != ''
    GROUP BY insight_tenant_id, source_id, day, read_id

),

admitted_reads AS (

    SELECT
        r.insight_tenant_id                             AS insight_tenant_id,
        r.source_id                                     AS source_id,
        r.day                                           AS day,
        r.read_id                                       AS read_id
    FROM read_state AS r
    LEFT JOIN first_reference AS f
           ON f.insight_tenant_id = r.insight_tenant_id
          AND f.source_id = r.source_id
    -- coalesce, not IS NULL: without join_use_nulls an unmatched LEFT JOIN
    -- yields the column's default rather than NULL, and both readings must
    -- give the same verdict.
    WHERE (r.insight_tenant_id, r.source_id, r.day, r.read_id, r.people) IN (
              SELECT insight_tenant_id, source_id, day, read_id, read_total_users
              FROM reference
          )
       OR coalesce(f.instance_has_references, 0) = 0
       OR r.read_id < f.since_read

),

-- The sessions/messages aggregate, keyed the way it publishes: user_id, never
-- email. It owns the credit split and the token split; the leaderboard owns
-- email, name and lines_added. Joined LEFT so a person-day the leaderboard
-- returned and this endpoint did not keeps its row instead of vanishing.
--
-- INVARIANT: no completeness gate here and none is owed. The endpoint is
-- unpaginated — page_size is ignored and `page` is rejected — so a read
-- returns every person in the range or fails outright, and the page-boundary
-- loss the leaderboard gate exists for cannot occur.
codex_sessions AS (

    SELECT
        coalesce(tenant_id, '')                         AS insight_tenant_id,
        coalesce(source_id, '')                         AS source_id,
        trim(user_id)                                   AS user_id,
        toDateOrNull(date)                              AS day,
        credit_total,
        on_demand_credits,
        toUInt64OrNull(toString(n_new_sessions_total))  AS n_new_sessions_total,
        toUInt64OrNull(toString(n_user_messages_total)) AS n_user_messages_total,
        toUInt64OrNull(toString(text_total_tokens))     AS text_total_tokens
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_codex_sessions_daily') }} FINAL
    WHERE user_id IS NOT NULL
      AND trim(user_id) != ''
      AND date IS NOT NULL

)

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
    -- The sessions_* keys come from chatgpt_team_codex_sessions_daily and are
    -- absent, not zero, when that endpoint did not return the person-day:
    -- seen_in_sessions says which, so a genuine zero stays distinguishable
    -- from an unread one. Money is NOT sourced from here — the cost pipeline
    -- reads the sessions stream directly, because this model's emission
    -- filter drops credit-bearing person-days by design.
    CAST(toJSONString(map(
        'credits',        toString(coalesce(credits, 0)),
        'n_turns',        toString(coalesce(toUInt32OrNull(toString(n_turns)), 0)),
        'text_tokens',    toString(coalesce(toUInt64OrNull(toString(text_tokens)), 0)),
        'current_streak', toString(coalesce(toUInt32OrNull(toString(current_streak)), 0)),
        'seen_in_sessions',       if(seen_in_sessions, '1', '0'),
        'sessions_credit_total',  if(seen_in_sessions, toString(coalesce(sessions_credit_total, 0)), ''),
        'sessions_on_demand_credits', if(seen_in_sessions, toString(coalesce(sessions_on_demand_credits, 0)), ''),
        'sessions_new_sessions',  if(seen_in_sessions, toString(coalesce(sessions_new_sessions, 0)), ''),
        'sessions_user_messages', if(seen_in_sessions, toString(coalesce(sessions_user_messages, 0)), ''),
        'sessions_text_total_tokens', if(seen_in_sessions, toString(coalesce(sessions_text_total_tokens, 0)), '')
    )) AS Nullable(String))                             AS tool_action_breakdown_json,
    'chatgpt_team'                                      AS source,
    data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version,
    -- The usage endpoint carries no seat lifecycle state.
    CAST(NULL AS Nullable(String))                      AS seat_status
FROM (
    -- The leaderboard is the spine and the sessions aggregate enriches it.
    -- LEFT, and joined on the id rather than the address: this endpoint keys
    -- on user_id and publishes no email, so an inner join would silently drop
    -- every leaderboard row whose user_id the vendor left null.
    SELECT
        lb.*,
        s.credit_total                      AS sessions_credit_total,
        s.on_demand_credits                 AS sessions_on_demand_credits,
        s.n_new_sessions_total              AS sessions_new_sessions,
        s.n_user_messages_total             AS sessions_user_messages,
        s.text_total_tokens                 AS sessions_text_total_tokens,
        -- Records the reconciliation outcome per person-day so a divergence
        -- between the two endpoints is queryable instead of reading as a
        -- column that happens to be null.
        s.day IS NOT NULL                   AS seen_in_sessions
    FROM (
    -- Bronze dedup: keep the latest ADMITTED extract per (email, date).
    -- INVARIANT: the gate runs inside this subquery. Deduping first would keep
    -- a short read's row and then reject it, dropping the day altogether
    -- instead of falling back to the last sound read.
    SELECT *
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_codex_user_daily') }}
    -- The read this row came from has to have brought the whole day. Rejected
    -- whole rather than row by row: a short read is short in an unknown place,
    -- so the people it DID return say nothing about the ones it did not.
    WHERE (coalesce(tenant_id, ''),
           coalesce(source_id, ''),
           toDateOrNull(date),
           JSONExtractInt(_airbyte_meta, 'sync_id')) IN (
              SELECT insight_tenant_id, source_id, day, read_id FROM admitted_reads
          )
    ORDER BY _airbyte_extracted_at DESC
    -- Dedup on the SAME normalized key the unique_key uses (lower(trim(email))),
    -- else two case-variant spellings of one address both survive and then
    -- collide on unique_key (unique-test failure).
    LIMIT 1 BY tenant_id, source_id, lower(trim(email)), date
    ) AS lb
    LEFT JOIN codex_sessions AS s
           ON coalesce(lb.tenant_id, '') = s.insight_tenant_id
          AND coalesce(lb.source_id, '') = s.source_id
          AND trim(coalesce(lb.user_id, '')) = s.user_id
          AND toDateOrNull(lb.date) = s.day
)
WHERE email IS NOT NULL
  AND trim(email) != ''
  AND date IS NOT NULL
  -- toDate throws on '' where it returns NULL on a NULL, so the emptiness
  -- guard is not redundant with the IS NOT NULL above.
  AND trim(date) != ''
  -- INVARIANT: every term is a counter this model emits into the class. The
  -- contract derives active_day from row existence, so a row admitted on a
  -- counter that only reaches tool_action_breakdown_json would assert activity
  -- none of its class columns can evidence — which is what
  -- assert_ai_dev_usage_rows_active fails on.
  AND (
        coalesce(toUInt32OrNull(toString(n_threads)), 0) > 0
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
