-- depends_on: {{ ref('claude_team__bronze_promoted') }}
-- Bronze → Silver step 1: Claude Team per-user per-day CC usage → class_ai_dev_usage
--
-- Source: bronze_claude_team.claude_team_code_metrics — daily aggregate stream
-- pulled via the customer-deployed claude-team-proxy from the claude.ai
-- web API (/api/organizations/{org_id}/claude_code/metrics). One row per
-- (email, metric_date) — metrics already aggregated to daily grain by the API.
--
-- Filters:
--   any activity counter > 0      — the class contract admits a person-day row
--                                   only for real activity
--   email IS NOT NULL / != ''     — rows without email cannot be attributed
--   metric_date IS NOT NULL       — guard against phantom 1970-01-01 rows
--
-- seat_status: `status`, carried rather than filtered on. The vendor restates
--   it on every read of the rolling window, so it describes the seat as of the
--   read, not as of `day` — filtering on it retroactively erases the whole
--   history of a person whose seat is later deactivated.
--
-- session_count / conversation_count: both `total_sessions`. The Team plan
--   reports no separate conversation counter — for Claude Code a session IS
--   the unit of conversation — so the same number feeds the class contract's
--   activity column and its conversations metric-feed column.
--   DQ note: a session counter can read 0 while lines_accepted > 0 —
--   Anthropic excludes headless / `cc -p` invocations from it. Not a model bug.
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

-- Completeness gate (#3172). The per-user endpoint pages an offset window over
-- sort_by=total_lines_accepted, and everyone with no accepted lines ties on it,
-- so a person can sit past the page boundary in one request and before it in
-- the next — returned by neither, while every request succeeds. A read that
-- lost somebody must not become the day's state, so it is dropped whole here
-- and the day keeps whatever the last sound read wrote.
WITH reference AS (

    -- The vendor's own headcount, as of the read that returned it. Keyed on the
    -- read and not the day: a day still in progress reports fewer people than it
    -- ends with, so judging an older read against a newer reference would reject
    -- sound reads.
    SELECT
        coalesce(tenant_id, '')                         AS insight_tenant_id,
        coalesce(source_id, '')                         AS source_id,
        toDate(metric_date)                             AS day,
        JSONExtractInt(_airbyte_meta, 'sync_id')        AS read_id,
        -- Not aliased `total_users`: the alias shadows the source column and
        -- ClickHouse then resolves the outer reference to the aggregate itself.
        max(toInt64OrNull(toString(total_users)))       AS read_total_users
    -- FINAL: the envelope table is a ReplacingMergeTree, so an unmerged part
    -- can still hold a superseded headcount for the same read.
    FROM {{ source('bronze_claude_team', 'claude_team_code_metrics_org') }} FINAL
    WHERE metric_date IS NOT NULL
      AND total_users IS NOT NULL
    GROUP BY insight_tenant_id, source_id, day, read_id

),

first_reference AS (

    -- Which read first carried a reference on this instance. Reads before it
    -- predate the field and cannot be judged; that read and everything after
    -- must carry one, or they are rejected — an unreferenced later read is the
    -- silent hole this gate exists to close.
    --
    -- The boundary is the READ, not its timestamp. Both streams are written by
    -- the same job, but not at the same instant: on the first sync after the
    -- upgrade the per-user rows can land a moment before the envelope that
    -- would judge them. A timestamp boundary would wave exactly those through
    -- as legacy.
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
        toDate(metric_date)                             AS day,
        JSONExtractInt(_airbyte_meta, 'sync_id')        AS read_id,
        -- Counts only rows the serving layer can attribute, so a headcount
        -- that includes someone arriving without an email fails the gate
        -- closed instead of passing on a body count.
        uniqExactIf(lower(trim(email)),
                    email IS NOT NULL AND trim(email) != '') AS people
    FROM {{ source('bronze_claude_team', 'claude_team_code_metrics') }}
    WHERE metric_date IS NOT NULL
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

)

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
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version,
    nullIf(lower(trim(coalesce(status, ''))), '')          AS seat_status
FROM {{ source('bronze_claude_team', 'claude_team_code_metrics') }}
WHERE email IS NOT NULL
  AND trim(email) != ''
  -- Guard against NULL metric_date: toDate(NULL) → 1970-01-01 silently
  -- corrupts the incremental boundary (same guard as cursor__ai_dev_usage).
  AND metric_date IS NOT NULL
  -- Activity gate over the same counters assert_ai_dev_usage_rows_active
  -- checks, so a roster row with no work on `day` never reaches the class.
  AND (coalesce(total_sessions, 0) > 0
       OR coalesce(total_lines_accepted, 0) > 0
       OR coalesce(toFloat64OrNull(toString(total_cost)), 0) > 0
       OR coalesce(prs_with_cc, 0) > 0
       OR coalesce(total_prs, 0) > 0)
  -- The read this row came from has to have brought the whole day. Rejected
  -- whole rather than row by row: a short read is short in an unknown place,
  -- so the people it DID return say nothing about the ones it did not.
  AND (coalesce(tenant_id, ''),
       coalesce(source_id, ''),
       toDate(metric_date),
       JSONExtractInt(_airbyte_meta, 'sync_id')) IN (
          SELECT insight_tenant_id, source_id, day, read_id FROM admitted_reads
      )
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
