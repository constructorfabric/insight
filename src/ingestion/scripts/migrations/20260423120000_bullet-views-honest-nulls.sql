-- Honest-null sweep for every remaining gold view that silently emits
-- zero in place of missing data. Consistent with:
--   20260422100000_ic-kpis-honest-nulls.sql (loc, prs_merged, pr_cycle_time_h)
--   20260422150000_team-member-honest-nulls.sql (dev_time_h, focus_time_pct, ai_loc_share_pct)
--
-- Two classes of fixes:
--
-- 1. "Source not ingested" — views emit a row per (person, date) from an
--    unrelated table (e.g. iterating bronze_cursor rows to fabricate
--    Claude Code / Codex / chatgpt rows). We flip those to NULL so the FE
--    bullet renders ComingSoon instead of a fake 0/91 bar.
--
--    ai_bullet_rows:          cc_active, cc_sessions, cc_lines,
--                             cc_tool_accept, cc_tool_acceptance,
--                             codex_active, chatgpt, claude_web
--    task_delivery_bullet_rows: task_reopen_rate
--    code_quality_bullet_rows: prs_per_dev, pr_cycle_time, build_success
--    exec_summary:            pr_cycle_time_h
--    team_member:             prs_merged
--    ic_chart_loc:            spec_lines
--
-- 2. "Denominator-zero fallback" — rate formulas had `if(denom > 0, ratio,
--    0)` which drags team averages down when a person has no activity to
--    compute a rate. NULL is honest: avg() in ClickHouse skips NULL.
--
--    task_delivery_bullet_rows: due_date_compliance, estimation_accuracy
--    ai_bullet_rows:          cursor_acceptance, ai_loc_share2
--    collab_bullet_rows:      slack_dm_ratio
--
-- Column-type cascades (Float64 → Nullable(Float64)):
--
--    code_quality_bullet_rows.metric_value
--    code_quality_person_period.v
--    code_quality_company_stats.*
--    exec_summary.ai_adoption_pct, ai_loc_share_pct, pr_cycle_time_h
--    team_member.prs_merged
--    ic_chart_loc.spec_lines
--
-- ai_company_stats — its hardcoded `toFloat64(0)` / `toFloat64(count())` in
-- the active_* family now becomes NULL when the upstream has no non-NULL
-- values (count(v) = 0).
--
-- Depends on 20260422000000_gold-views.sql plus the two prior honest-null
-- migrations. Order: DROP downstream before upstream, re-CREATE upstream
-- first.

-- =====================================================================
-- DROP in reverse dependency order
-- =====================================================================
DROP VIEW IF EXISTS insight.ic_chart_loc;
DROP VIEW IF EXISTS insight.team_member;
DROP VIEW IF EXISTS insight.exec_summary;
DROP VIEW IF EXISTS insight.ai_company_stats;
DROP VIEW IF EXISTS insight.code_quality_company_stats;
DROP VIEW IF EXISTS insight.task_delivery_company_stats;
DROP VIEW IF EXISTS insight.collab_company_stats;
DROP VIEW IF EXISTS insight.ai_person_period;
DROP VIEW IF EXISTS insight.code_quality_person_period;
DROP VIEW IF EXISTS insight.task_delivery_person_period;
DROP VIEW IF EXISTS insight.collab_person_period;
DROP VIEW IF EXISTS insight.ai_bullet_rows;
DROP VIEW IF EXISTS insight.code_quality_bullet_rows;
DROP VIEW IF EXISTS insight.task_delivery_bullet_rows;
DROP VIEW IF EXISTS insight.collab_bullet_rows;

-- =====================================================================
-- collab_bullet_rows — slack_dm_ratio emits NULL when poster sent 0 msgs
-- =====================================================================
CREATE VIEW insight.collab_bullet_rows AS
SELECT
    lower(e.email)                                                AS person_id,
    p.org_unit_id,
    toString(e.date)                                              AS metric_date,
    'm365_emails_sent'                                            AS metric_key,
    toFloat64(ifNull(e.sent_count, 0))                            AS metric_value
FROM silver.class_collab_email_activity AS e
LEFT JOIN insight.people AS p ON lower(e.email) = p.person_id
UNION ALL
SELECT
    lower(m.email), p.org_unit_id, toString(m.date), 'zoom_calls',
    toFloat64(ifNull(m.meetings_attended, 0))
FROM silver.class_collab_meeting_activity AS m
LEFT JOIN insight.people AS p ON lower(m.email) = p.person_id
WHERE m.data_source = 'insight_zoom'
UNION ALL
SELECT
    f.email, p.org_unit_id, toString(f.day), 'meeting_hours',
    least(toFloat64(ifNull(f.meeting_hours, 0)), f.working_hours_per_day)
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, toString(c.date), 'm365_teams_messages',
    toFloat64(c.total_chat_messages)
FROM silver.class_collab_chat_activity AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(d.email), p.org_unit_id, toString(d.date), 'm365_files_shared',
    toFloat64(ifNull(d.shared_internally_count, 0)) +
    toFloat64(ifNull(d.shared_externally_count, 0))
FROM silver.class_collab_document_activity AS d
LEFT JOIN insight.people AS p ON lower(d.email) = p.person_id
UNION ALL
SELECT
    f.email, p.org_unit_id, toString(f.day), 'meeting_free',
    if(ifNull(f.meeting_hours, 0) = 0, toFloat64(1), toFloat64(0))
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id
UNION ALL
SELECT
    lower(s.email_address), p.org_unit_id, s.date, 'slack_thread_participation',
    toFloat64(ifNull(s.channel_messages_posted_count, 0))
FROM bronze_slack.users_details AS s
LEFT JOIN insight.people AS p ON lower(s.email_address) = p.person_id
UNION ALL
SELECT
    lower(s.email_address), p.org_unit_id, s.date, 'slack_message_engagement',
    toFloat64(ifNull(s.messages_posted_count, 0))
FROM bronze_slack.users_details AS s
LEFT JOIN insight.people AS p ON lower(s.email_address) = p.person_id
UNION ALL
SELECT
    lower(s.email_address), p.org_unit_id, s.date, 'slack_dm_ratio',
    -- 0 messages posted — no activity to compute a rate on. NULL, not 0.
    if(ifNull(s.messages_posted_count, 0) > 0,
       round(((toFloat64(ifNull(s.messages_posted_count, 0)) -
               toFloat64(ifNull(s.channel_messages_posted_count, 0))) /
              toFloat64(s.messages_posted_count)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM bronze_slack.users_details AS s
LEFT JOIN insight.people AS p ON lower(s.email_address) = p.person_id;

-- =====================================================================
-- task_delivery_bullet_rows — NULL for unsourced + rate fallbacks
-- =====================================================================
CREATE VIEW insight.task_delivery_bullet_rows AS
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date)                                       AS metric_date,
    'tasks_completed'                                             AS metric_key,
    toFloat64(j.tasks_closed)                                     AS metric_value
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id, p.org_unit_id, toString(j.metric_date), 'task_dev_time',
    round(ifNull(j.avg_time_spent, 0) / 3600., 1)
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id, p.org_unit_id, toString(j.metric_date), 'task_reopen_rate',
    -- No source ingested — NULL, not fabricated 0.
    CAST(NULL AS Nullable(Float64))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id, p.org_unit_id, toString(j.metric_date), 'due_date_compliance',
    -- No tasks with a due date = "cannot compute compliance", not 0% compliance.
    if(j.has_due_date_count > 0,
       round((toFloat64(j.on_time_count) / toFloat64(j.has_due_date_count)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id, p.org_unit_id, toString(j.metric_date), 'estimation_accuracy',
    -- No spent-time or no estimate = "cannot compute", not 0% accurate.
    if((ifNull(j.avg_time_spent, 0) > 0) AND (j.avg_time_estimate IS NOT NULL),
       round((j.avg_time_estimate / j.avg_time_spent) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id;

-- =====================================================================
-- code_quality_bullet_rows — metric_value Nullable, 3 sources → NULL
-- =====================================================================
CREATE VIEW insight.code_quality_bullet_rows
(
    `person_id`    String,
    `org_unit_id`  Nullable(String),
    `metric_date`  String,
    `metric_key`   String,
    `metric_value` Nullable(Float64)
)
AS SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date)                                       AS metric_date,
    'bugs_fixed'                                                  AS metric_key,
    CAST(toFloat64(j.bugs_fixed) AS Nullable(Float64))            AS metric_value
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id, p.org_unit_id, toString(j.metric_date), 'prs_per_dev',
    -- Bitbucket PR ingestion not wired.
    CAST(NULL AS Nullable(Float64))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id, p.org_unit_id, toString(j.metric_date), 'pr_cycle_time',
    CAST(NULL AS Nullable(Float64))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
UNION ALL
SELECT
    j.person_id, p.org_unit_id, toString(j.metric_date), 'build_success',
    -- CI build results not ingested.
    CAST(NULL AS Nullable(Float64))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id;

-- =====================================================================
-- ai_bullet_rows — 8 unsourced metrics → NULL, 2 rate fallbacks → NULL
-- =====================================================================
CREATE VIEW insight.ai_bullet_rows AS
SELECT
    lower(c.email)                                                AS person_id,
    p.org_unit_id,
    c.day                                                         AS metric_date,
    'active_ai_members'                                           AS metric_key,
    if(c.isActive = true, toFloat64(1), toFloat64(0))             AS metric_value
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_active',
    if(c.isActive = true, toFloat64(1), toFloat64(0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_active',
    -- Claude Code Enterprise API not ingested.
    CAST(NULL AS Nullable(Float64))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'codex_active',
    -- Codex not ingested.
    CAST(NULL AS Nullable(Float64))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'team_ai_loc',
    toFloat64(ifNull(c.acceptedLinesAdded, 0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_acceptance',
    -- 0 completions shown — no activity to compute an acceptance rate on.
    if(toFloat64(ifNull(c.totalTabsShown, 0)) > 0,
       round((toFloat64(ifNull(c.totalTabsAccepted, 0)) /
              toFloat64(c.totalTabsShown)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_tool_acceptance',
    CAST(NULL AS Nullable(Float64))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_completions',
    toFloat64(ifNull(c.totalTabsShown, 0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_agents',
    toFloat64(ifNull(c.agentRequests, 0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_lines',
    toFloat64(ifNull(c.acceptedLinesAdded, 0))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_sessions',
    CAST(NULL AS Nullable(Float64))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_tool_accept',
    CAST(NULL AS Nullable(Float64))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_lines',
    CAST(NULL AS Nullable(Float64))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'ai_loc_share2',
    -- 0 total lines written — no denominator for AI share.
    if(toFloat64(ifNull(c.totalLinesAdded, 0)) > 0,
       round((toFloat64(ifNull(c.acceptedLinesAdded, 0)) /
              toFloat64(c.totalLinesAdded)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'claude_web',
    -- Claude.ai web not ingested.
    CAST(NULL AS Nullable(Float64))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'chatgpt',
    -- ChatGPT Team not ingested.
    CAST(NULL AS Nullable(Float64))
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id;

-- =====================================================================
-- *_person_period — unchanged aggregation, just rebuilt after DROP.
-- ClickHouse views aren't late-binding; downstream must be re-CREATEd
-- whenever we rebuild the upstream view.
-- =====================================================================
CREATE VIEW insight.collab_person_period AS
SELECT
    metric_key,
    person_id,
    any(org_unit_id)                                              AS org_unit_id,
    max(metric_date)                                              AS metric_date,
    multiIf(
        metric_key IN ('m365_emails_sent','zoom_calls','meeting_hours',
                       'm365_teams_messages','m365_files_shared','meeting_free',
                       'slack_thread_participation','slack_message_engagement'),
        sum(metric_value),
        avg(metric_value))                                        AS v
FROM insight.collab_bullet_rows
GROUP BY metric_key, person_id;

CREATE VIEW insight.task_delivery_person_period AS
SELECT
    metric_key,
    person_id,
    any(org_unit_id)                                              AS org_unit_id,
    max(metric_date)                                              AS metric_date,
    multiIf(
        metric_key = 'tasks_completed',   sum(metric_value),
        metric_key = 'estimation_accuracy',
            -- Accuracy is folded: closer to 100 is better. avg of |100 - v|
            -- across valid samples, then subtracted from 100 and clamped ≥0.
            if(countIf((metric_value > 0) AND (metric_value <= 200)) > 0,
               greatest(toFloat64(0),
                        toFloat64(100) -
                        avgIf(abs(toFloat64(100) - metric_value),
                              (metric_value > 0) AND (metric_value <= 200))),
               NULL),
        avg(metric_value))                                        AS v
FROM insight.task_delivery_bullet_rows
GROUP BY metric_key, person_id;

CREATE VIEW insight.code_quality_person_period
(
    `metric_key`  String,
    `person_id`   String,
    `org_unit_id` Nullable(String),
    `metric_date` String,
    `v`           Nullable(Float64)
)
AS SELECT
    metric_key,
    person_id,
    any(org_unit_id)                                              AS org_unit_id,
    max(metric_date)                                              AS metric_date,
    multiIf(metric_key IN ('bugs_fixed','prs_per_dev'),
            sum(metric_value),
            avg(metric_value))                                    AS v
FROM insight.code_quality_bullet_rows
GROUP BY metric_key, person_id;

CREATE VIEW insight.ai_person_period AS
SELECT
    metric_key,
    person_id,
    any(org_unit_id)                                              AS org_unit_id,
    max(metric_date)                                              AS metric_date,
    multiIf(
        metric_key IN ('chatgpt','cc_lines','cc_sessions','cursor_agents',
                       'cursor_lines','claude_web','cursor_completions','team_ai_loc'),
        sum(metric_value),
        metric_key IN ('active_ai_members','cursor_active','cc_active','codex_active'),
        max(metric_value),
        avg(metric_value))                                        AS v
FROM insight.ai_bullet_rows
GROUP BY metric_key, person_id;

-- =====================================================================
-- *_company_stats — rebuilt. code_quality_* columns Nullable.
-- ai_company_stats: the active_* family's hardcoded floor/ceiling becomes
-- NULL when no non-NULL metric values exist.
-- =====================================================================
CREATE VIEW insight.collab_company_stats AS
SELECT
    metric_key,
    avg(v)                    AS company_value,
    quantileExact(0.5)(v)     AS company_median,
    min(v)                    AS company_p5,
    max(v)                    AS company_p95
FROM insight.collab_person_period
GROUP BY metric_key;

CREATE VIEW insight.task_delivery_company_stats AS
SELECT
    metric_key,
    avg(v)                    AS company_value,
    quantileExact(0.5)(v)     AS company_median,
    min(v)                    AS company_p5,
    max(v)                    AS company_p95
FROM insight.task_delivery_person_period
GROUP BY metric_key;

CREATE VIEW insight.code_quality_company_stats
(
    `metric_key`      String,
    `company_value`   Nullable(Float64),
    `company_median`  Nullable(Float64),
    `company_p5`      Nullable(Float64),
    `company_p95`     Nullable(Float64)
)
AS SELECT
    metric_key,
    avg(v)                    AS company_value,
    quantileExact(0.5)(v)     AS company_median,
    min(v)                    AS company_p5,
    max(v)                    AS company_p95
FROM insight.code_quality_person_period
GROUP BY metric_key;

CREATE VIEW insight.ai_company_stats AS
SELECT
    metric_key,
    multiIf(
        metric_key IN ('active_ai_members','cursor_active','cc_active','codex_active'),
        sum(v),
        avg(v))                                                   AS company_value,
    -- For active_* metrics the "distribution" was synthetic: median/p5 pinned
    -- to 0 and p95 pinned to team headcount so the bullet reads "N / team".
    -- When the upstream has no non-NULL values (e.g. cc_active), collapse
    -- the synthetic distribution to NULL so the bullet renders ComingSoon.
    multiIf(
        metric_key IN ('active_ai_members','cursor_active','cc_active','codex_active'),
        if(count(v) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)),
        quantileExact(0.5)(v))                                    AS company_median,
    multiIf(
        metric_key IN ('active_ai_members','cursor_active','cc_active','codex_active'),
        if(count(v) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)),
        min(v))                                                   AS company_p5,
    multiIf(
        metric_key IN ('active_ai_members','cursor_active','cc_active','codex_active'),
        if(count(v) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())),
        max(v))                                                   AS company_p95
FROM insight.ai_person_period
GROUP BY metric_key;

-- =====================================================================
-- exec_summary — Nullable pr_cycle_time_h; ai_* NULL when no cursor rows
-- for the org/day (LEFT JOIN miss, not "cursor there but inactive").
-- =====================================================================
CREATE VIEW insight.exec_summary
(
    `org_unit_id`       Nullable(String),
    `org_unit_name`     Nullable(String),
    `headcount`         UInt32,
    `tasks_closed`      UInt64,
    `bugs_fixed`        UInt64,
    `build_success_pct` Nullable(Float64),
    `focus_time_pct`    Nullable(Float64),
    `ai_adoption_pct`   Nullable(Float64),
    `ai_loc_share_pct`  Nullable(Float64),
    `pr_cycle_time_h`   Nullable(Float64),
    `metric_date`       Nullable(String)
)
AS SELECT
    base.org_unit_id                                              AS org_unit_id,
    base.org_unit_name                                            AS org_unit_name,
    org.headcount                                                 AS headcount,
    ifNull(j.tasks_closed, 0)                                     AS tasks_closed,
    ifNull(j.bugs_fixed, 0)                                       AS bugs_fixed,
    CAST(NULL AS Nullable(Float64))                               AS build_success_pct,
    greatest(0, least(100, round(base.avg_focus_pct, 1)))         AS focus_time_pct,
    -- Cursor row missing for this (org, day) → NULL, not 0%.
    if(ai.active_count IS NULL,
       CAST(NULL AS Nullable(Float64)),
       round((ai.active_count * 100.) / greatest(org.headcount, 1), 1)) AS ai_adoption_pct,
    if(ai.avg_ai_loc_share IS NULL,
       CAST(NULL AS Nullable(Float64)),
       round(ai.avg_ai_loc_share, 1))                             AS ai_loc_share_pct,
    -- Bitbucket PR cycle time not ingested.
    CAST(NULL AS Nullable(Float64))                               AS pr_cycle_time_h,
    base.metric_date                                              AS metric_date
FROM
(
    SELECT
        pe.org_unit_id,
        any(pe.org_unit_name)         AS org_unit_name,
        toString(f.day)               AS metric_date,
        avg(f.focus_time_pct)         AS avg_focus_pct
    FROM silver.class_focus_metrics AS f
    INNER JOIN insight.people AS pe
        ON (f.email = pe.person_id) AND (pe.status = 'Active')
    GROUP BY pe.org_unit_id, f.day
) AS base
INNER JOIN
(
    SELECT org_unit_id, toUInt32(count()) AS headcount
    FROM insight.people
    WHERE status = 'Active'
    GROUP BY org_unit_id
) AS org ON base.org_unit_id = org.org_unit_id
LEFT JOIN
(
    SELECT
        pe.org_unit_id,
        toString(j.metric_date)       AS metric_date,
        sum(j.tasks_closed)           AS tasks_closed,
        sum(j.bugs_fixed)             AS bugs_fixed
    FROM insight.jira_closed_tasks AS j
    INNER JOIN insight.people AS pe
        ON (j.person_id = pe.person_id) AND (pe.status = 'Active')
    GROUP BY pe.org_unit_id, j.metric_date
) AS j
    ON (base.org_unit_id = j.org_unit_id) AND (base.metric_date = j.metric_date)
LEFT JOIN
(
    SELECT
        pe.org_unit_id,
        c.day                         AS metric_date,
        countDistinctIf(lower(c.email), c.isActive = true)                      AS active_count,
        avgIf(if(toFloat64(ifNull(c.totalLinesAdded, 0)) > 0,
                 (toFloat64(ifNull(c.acceptedLinesAdded, 0)) /
                  toFloat64(c.totalLinesAdded)) * 100,
                 0),
              c.isActive = true)                                                AS avg_ai_loc_share
    FROM bronze_cursor.cursor_daily_usage AS c
    INNER JOIN insight.people AS pe
        ON (lower(c.email) = pe.person_id) AND (pe.status = 'Active')
    GROUP BY pe.org_unit_id, c.day
) AS ai
    ON (base.org_unit_id = ai.org_unit_id) AND (base.metric_date = ai.metric_date);

-- =====================================================================
-- team_member — prs_merged → Nullable NULL (unsourced).
-- Re-state all other honest-null fixes from 20260422150000 so this view
-- stays consistent if this migration runs standalone on a snapshot that
-- doesn't have the prior patch applied.
-- =====================================================================
CREATE VIEW insight.team_member AS
SELECT
    p.person_id                                                   AS person_id,
    p.display_name                                                AS display_name,
    p.seniority                                                   AS seniority,
    p.org_unit_id                                                 AS org_unit_id,
    toFloat64(ifNull(j.tasks_closed, 0))                          AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0))                            AS bugs_fixed,
    if(f.dev_time_h IS NULL,
        CAST(NULL AS Nullable(Float64)),
        CAST(greatest(0, round(f.dev_time_h, 1)) AS Nullable(Float64)))         AS dev_time_h,
    -- Bitbucket PR ingestion not wired.
    CAST(NULL AS Nullable(Float64))                               AS prs_merged,
    CAST(NULL AS Nullable(Float64))                               AS build_success_pct,
    if(f.focus_time_pct IS NULL,
        CAST(NULL AS Nullable(Float64)),
        CAST(greatest(0, least(100, round(f.focus_time_pct, 1)))
             AS Nullable(Float64)))                                             AS focus_time_pct,
    -- ai_tools stays non-Nullable (CH forbids Nullable(Array(T))). FE gates
    -- AI-related alerts via data_availability.ai instead of per-row nulls.
    if(ifNull(cur.is_active, 0) = 1,
       ['Cursor'],
       CAST([] AS Array(String)))                                 AS ai_tools,
    if(cur.person_id IS NULL,
        CAST(NULL AS Nullable(Float64)),
        CAST(round(cur.ai_loc_share_pct, 1) AS Nullable(Float64)))              AS ai_loc_share_pct,
    f.metric_date                                                 AS metric_date
FROM insight.people AS p
INNER JOIN
(
    SELECT
        email,
        toString(day)             AS metric_date,
        focus_time_pct,
        dev_time_h
    FROM silver.class_focus_metrics
) AS f ON p.person_id = f.email
LEFT JOIN
(
    SELECT
        person_id,
        toString(metric_date)     AS metric_date,
        sum(tasks_closed)         AS tasks_closed,
        sum(bugs_fixed)           AS bugs_fixed
    FROM insight.jira_closed_tasks
    GROUP BY person_id, metric_date
) AS j ON (p.person_id = j.person_id) AND (f.metric_date = j.metric_date)
LEFT JOIN
(
    SELECT
        lower(email)              AS person_id,
        day                       AS metric_date,
        if(isActive = true, 1, 0) AS is_active,
        if(toFloat64(ifNull(totalLinesAdded, 0)) > 0,
           round((toFloat64(ifNull(acceptedLinesAdded, 0)) /
                  toFloat64(totalLinesAdded)) * 100, 1),
           0)                     AS ai_loc_share_pct
    FROM bronze_cursor.cursor_daily_usage
) AS cur ON (p.person_id = cur.person_id) AND (f.metric_date = cur.metric_date)
WHERE p.status = 'Active';

-- =====================================================================
-- ic_chart_loc — spec_lines → Nullable NULL (spec extractor not wired)
-- =====================================================================
CREATE VIEW insight.ic_chart_loc
(
    `person_id`   Nullable(String),
    `org_unit_id` Nullable(String),
    `date_bucket` Nullable(String),
    `metric_date` Nullable(String),
    `ai_loc`      Float64,
    `code_loc`    Float64,
    `spec_lines`  Nullable(Float64)
)
AS SELECT
    lower(c.email)                                                AS person_id,
    p.org_unit_id,
    toString(toStartOfWeek(toDate(c.day)))                        AS date_bucket,
    toString(toStartOfWeek(toDate(c.day)))                        AS metric_date,
    toFloat64(sum(ifNull(c.acceptedLinesAdded, 0)))               AS ai_loc,
    toFloat64(sum(ifNull(c.totalLinesAdded, 0)) -
              sum(ifNull(c.acceptedLinesAdded, 0)))               AS code_loc,
    CAST(NULL AS Nullable(Float64))                               AS spec_lines
FROM bronze_cursor.cursor_daily_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
GROUP BY
    lower(c.email),
    p.org_unit_id,
    toStartOfWeek(toDate(c.day));
