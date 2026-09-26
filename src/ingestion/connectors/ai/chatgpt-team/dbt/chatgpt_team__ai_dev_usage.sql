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
    incremental_strategy='delete+insert',
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
-- INVARIANT: no completeness gate here, because nothing in the response could
-- drive one. The endpoint is unpaginated — page_size is ignored and `page` is
-- rejected — which proves only that a page-boundary loss cannot occur, there
-- being no pages. It publishes no envelope and no expected total, so a partial
-- answer is undetectable from its own payload. That is also why the
-- authoritative total stays with the leaderboard, which can be judged.
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
        toUInt64OrNull(toString(n_tasks_web))           AS n_tasks_web,
        toUInt64OrNull(toString(n_code_reviews_web))    AS n_code_reviews_web,
        toUInt64OrNull(toString(uncached_text_input_tokens)) AS uncached_text_input_tokens,
        toUInt64OrNull(toString(cached_text_input_tokens))   AS cached_text_input_tokens,
        toUInt64OrNull(toString(text_output_tokens))    AS text_output_tokens,
        toUInt64OrNull(toString(text_total_tokens))     AS text_total_tokens,
        -- Per-surface breakdown, the reason this endpoint was added.
        credit_cli,
        credit_vscode,
        credit_exec,
        credit_sdk_ts,
        credit_desktop,
        credit_web,
        credit_slack,
        credit_github_code_review,
        credit_github_turn,
        toUInt64OrNull(toString(n_new_sessions_cli))  AS n_new_sessions_cli,
        toUInt64OrNull(toString(n_new_sessions_vscode))  AS n_new_sessions_vscode,
        toUInt64OrNull(toString(n_new_sessions_exec))  AS n_new_sessions_exec,
        toUInt64OrNull(toString(n_new_sessions_sdk_ts))  AS n_new_sessions_sdk_ts,
        toUInt64OrNull(toString(n_new_sessions_desktop))  AS n_new_sessions_desktop,
        toUInt64OrNull(toString(n_new_sessions_work_desktop))  AS n_new_sessions_work_desktop,
        toUInt64OrNull(toString(n_new_sessions_work_web))  AS n_new_sessions_work_web,
        toUInt64OrNull(toString(n_new_sessions_work_mobile))  AS n_new_sessions_work_mobile,
        toUInt64OrNull(toString(n_new_sessions_other))  AS n_new_sessions_other,
        toUInt64OrNull(toString(n_user_messages_cli)) AS n_user_messages_cli,
        toUInt64OrNull(toString(n_user_messages_vscode)) AS n_user_messages_vscode,
        toUInt64OrNull(toString(n_user_messages_exec)) AS n_user_messages_exec,
        toUInt64OrNull(toString(n_user_messages_sdk_ts)) AS n_user_messages_sdk_ts,
        toUInt64OrNull(toString(n_user_messages_desktop)) AS n_user_messages_desktop,
        toUInt64OrNull(toString(n_user_messages_work_desktop)) AS n_user_messages_work_desktop,
        toUInt64OrNull(toString(n_user_messages_work_web)) AS n_user_messages_work_web,
        toUInt64OrNull(toString(n_user_messages_work_mobile)) AS n_user_messages_work_mobile,
        toUInt64OrNull(toString(n_user_messages_other)) AS n_user_messages_other
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
    -- Codex threads ≈ coding sessions. Non-nullable UInt32 per contract, which
    -- is the one place a failed parse cannot stay unknown: the column admits no
    -- NULL, so an unparseable counter reads as 0 here. It cannot admit the row
    -- on its own, though — the emission filter tests the same expression, and a
    -- value that did not parse fails it. conversation_count below is nullable
    -- and keeps the distinction.
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
    --
    -- Every counter key reads '' unless it holds a whole non-negative number,
    -- and that one rule covers three different absences: the endpoint did not
    -- return the person-day, or it returned no value, or it returned one that
    -- is not a count. None of them is a zero, and '0' therefore always means a
    -- measured zero. seen_in_sessions separates the first case from the other
    -- two, so a reader can tell "not read" from "read and unusable". Money is NOT sourced from here: the cost path reads
    -- the leaderboard's credits through class_ai_credit_usage, because this
    -- model's emission filter drops credit-bearing person-days by design. The
    -- sessions figures beside them are the breakdown and the reconciliation,
    -- never the amount anything is charged on.
    CAST(toJSONString(map(
        'credits',        toString(coalesce(credits, 0)),
        -- '' where the value does not parse as a whole non-negative number,
        -- never '0': these columns are Nullable(Decimal(38, 9)) because the
        -- vendor publishes them as JSON numbers, and toString of one trims its
        -- trailing zeros — so an integral reading converts exactly, while a
        -- fractional or negative one yields NULL. Writing that NULL as 0 would
        -- report a measurement of none where there is no measurement.
        'n_turns',        ifNull(toString(toUInt32OrNull(toString(n_turns))), ''),
        'text_tokens',    ifNull(toString(toUInt64OrNull(toString(text_tokens))), ''),
        'current_streak', ifNull(toString(toUInt32OrNull(toString(current_streak))), ''),
        'seen_in_sessions',       if(seen_in_sessions, '1', '0'),
        'sessions_credit_total',  if(seen_in_sessions, toString(coalesce(sessions_credit_total, 0)), ''),
        'sessions_on_demand_credits', if(seen_in_sessions, toString(coalesce(sessions_on_demand_credits, 0)), ''),
        'sessions_new_sessions',  ifNull(toString(sessions_new_sessions), ''),
        'sessions_user_messages', ifNull(toString(sessions_user_messages), ''),
        'sessions_tasks_web',     ifNull(toString(sessions_tasks_web), ''),
        'sessions_code_reviews_web', ifNull(toString(sessions_code_reviews_web), ''),
        'sessions_uncached_input_tokens', ifNull(toString(sessions_uncached_input_tokens), ''),
        'sessions_cached_input_tokens', ifNull(toString(sessions_cached_input_tokens), ''),
        'sessions_output_tokens', ifNull(toString(sessions_output_tokens), ''),
        'sessions_text_total_tokens', ifNull(toString(sessions_text_total_tokens), ''),
        'sessions_credit_cli', if(seen_in_sessions, toString(coalesce(sessions_credit_cli, 0)), ''),
        'sessions_credit_vscode', if(seen_in_sessions, toString(coalesce(sessions_credit_vscode, 0)), ''),
        'sessions_credit_exec', if(seen_in_sessions, toString(coalesce(sessions_credit_exec, 0)), ''),
        'sessions_credit_sdk_ts', if(seen_in_sessions, toString(coalesce(sessions_credit_sdk_ts, 0)), ''),
        'sessions_credit_desktop', if(seen_in_sessions, toString(coalesce(sessions_credit_desktop, 0)), ''),
        'sessions_credit_web', if(seen_in_sessions, toString(coalesce(sessions_credit_web, 0)), ''),
        'sessions_credit_slack', if(seen_in_sessions, toString(coalesce(sessions_credit_slack, 0)), ''),
        'sessions_credit_github_code_review', if(seen_in_sessions, toString(coalesce(sessions_credit_github_code_review, 0)), ''),
        'sessions_credit_github_turn', if(seen_in_sessions, toString(coalesce(sessions_credit_github_turn, 0)), ''),
        'sessions_new_sessions_cli', ifNull(toString(sessions_new_sessions_cli), ''),
        'sessions_new_sessions_vscode', ifNull(toString(sessions_new_sessions_vscode), ''),
        'sessions_new_sessions_exec', ifNull(toString(sessions_new_sessions_exec), ''),
        'sessions_new_sessions_sdk_ts', ifNull(toString(sessions_new_sessions_sdk_ts), ''),
        'sessions_new_sessions_desktop', ifNull(toString(sessions_new_sessions_desktop), ''),
        'sessions_new_sessions_work_desktop', ifNull(toString(sessions_new_sessions_work_desktop), ''),
        'sessions_new_sessions_work_web', ifNull(toString(sessions_new_sessions_work_web), ''),
        'sessions_new_sessions_work_mobile', ifNull(toString(sessions_new_sessions_work_mobile), ''),
        'sessions_new_sessions_other', ifNull(toString(sessions_new_sessions_other), ''),
        'sessions_user_messages_cli', ifNull(toString(sessions_user_messages_cli), ''),
        'sessions_user_messages_vscode', ifNull(toString(sessions_user_messages_vscode), ''),
        'sessions_user_messages_exec', ifNull(toString(sessions_user_messages_exec), ''),
        'sessions_user_messages_sdk_ts', ifNull(toString(sessions_user_messages_sdk_ts), ''),
        'sessions_user_messages_desktop', ifNull(toString(sessions_user_messages_desktop), ''),
        'sessions_user_messages_work_desktop', ifNull(toString(sessions_user_messages_work_desktop), ''),
        'sessions_user_messages_work_web', ifNull(toString(sessions_user_messages_work_web), ''),
        'sessions_user_messages_work_mobile', ifNull(toString(sessions_user_messages_work_mobile), ''),
        'sessions_user_messages_other', ifNull(toString(sessions_user_messages_other), '')
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
        s.n_tasks_web                       AS sessions_tasks_web,
        s.n_code_reviews_web                AS sessions_code_reviews_web,
        s.uncached_text_input_tokens        AS sessions_uncached_input_tokens,
        s.cached_text_input_tokens          AS sessions_cached_input_tokens,
        s.text_output_tokens                AS sessions_output_tokens,
        s.text_total_tokens                 AS sessions_text_total_tokens,
        s.credit_cli AS sessions_credit_cli,
        s.credit_vscode AS sessions_credit_vscode,
        s.credit_exec AS sessions_credit_exec,
        s.credit_sdk_ts AS sessions_credit_sdk_ts,
        s.credit_desktop AS sessions_credit_desktop,
        s.credit_web AS sessions_credit_web,
        s.credit_slack AS sessions_credit_slack,
        s.credit_github_code_review AS sessions_credit_github_code_review,
        s.credit_github_turn AS sessions_credit_github_turn,
        s.n_new_sessions_cli AS sessions_new_sessions_cli,
        s.n_new_sessions_vscode AS sessions_new_sessions_vscode,
        s.n_new_sessions_exec AS sessions_new_sessions_exec,
        s.n_new_sessions_sdk_ts AS sessions_new_sessions_sdk_ts,
        s.n_new_sessions_desktop AS sessions_new_sessions_desktop,
        s.n_new_sessions_work_desktop AS sessions_new_sessions_work_desktop,
        s.n_new_sessions_work_web AS sessions_new_sessions_work_web,
        s.n_new_sessions_work_mobile AS sessions_new_sessions_work_mobile,
        s.n_new_sessions_other AS sessions_new_sessions_other,
        s.n_user_messages_cli AS sessions_user_messages_cli,
        s.n_user_messages_vscode AS sessions_user_messages_vscode,
        s.n_user_messages_exec AS sessions_user_messages_exec,
        s.n_user_messages_sdk_ts AS sessions_user_messages_sdk_ts,
        s.n_user_messages_desktop AS sessions_user_messages_desktop,
        s.n_user_messages_work_desktop AS sessions_user_messages_work_desktop,
        s.n_user_messages_work_web AS sessions_user_messages_work_web,
        s.n_user_messages_work_mobile AS sessions_user_messages_work_mobile,
        s.n_user_messages_other AS sessions_user_messages_other,
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
