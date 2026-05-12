-- =====================================================================
-- Insight gold views — read from silver instead of bronze
-- =====================================================================
--
-- Bronze tables are append-only archives — Airbyte writes every sync's
-- rows fresh, so a single business entity accumulates ×N copies. The
-- earlier gold views in 20260422000000_gold-views.sql aggregated bronze
-- directly — `count()`/`sum()` on duplicated rows = inflated metrics
-- visible in the backend.
--
-- This migration rewrites every gold view that has a clean silver
-- equivalent so it reads from silver. Silver dedupes via
-- ReplacingMergeTree(_version) (after #237) so each business entity
-- contributes once.
--
-- Where pre-aggregated metric tables already exist (silver.mtr_git_person_*
-- from PR #198), the views pass through those instead of re-aggregating
-- class_git_*. Affects: commits_daily, ic_chart_delivery (commits/prs_merged),
-- ic_kpis (loc/prs_merged/pr_cycle_time_h), ic_chart_loc (spec_lines).
--
-- The only intentional bronze reference left is `insight.people`, which
-- already deduplicates via argMax(_airbyte_extracted_at) + GROUP BY
-- person_id and is therefore correct on raw bronze.
-- =====================================================================

-- ---------------------------------------------------------------------
-- commits_daily ← silver.mtr_git_person_weekly  (pre-aggregated, PR #198)
-- ---------------------------------------------------------------------
-- Reads pre-aggregated weekly commit counts directly — no in-view count()
-- aggregation needed. metric_date carries the week-start date (Monday).
-- The only downstream consumer (insight.ic_chart_delivery, also rewritten
-- in this PR) bucketizes by week and is grain-compatible.
-- Tenant scoping via INNER JOIN insight.people (status='Active'), not by
-- hardcoded email-domain filter.
DROP VIEW IF EXISTS insight.commits_daily;
CREATE VIEW insight.commits_daily AS
SELECT
    m.person_key                              AS person_id,
    m.week                                    AS metric_date,
    toUInt64(m.commits)                       AS commits
FROM silver.mtr_git_person_weekly AS m
INNER JOIN insight.people AS p
    ON m.person_key = p.person_id
WHERE p.status = 'Active'
  AND m.week IS NOT NULL;

-- ---------------------------------------------------------------------
-- zoom_person_daily ← silver.class_collab_meeting_activity
-- ---------------------------------------------------------------------
DROP VIEW IF EXISTS insight.zoom_person_daily;
CREATE VIEW insight.zoom_person_daily AS
SELECT
    lower(email)                              AS person_id,
    date                                      AS metric_date,
    lower(email)                              AS user_email,
    toUInt64(coalesce(calls_count, 0))        AS zoom_calls,
    toFloat64(coalesce(audio_duration_seconds, 0)) / 3600.0
                                              AS meeting_hours
FROM silver.class_collab_meeting_activity
WHERE data_source = 'insight_zoom'
  AND email IS NOT NULL
  AND email != '';

-- ---------------------------------------------------------------------
-- teams_person_daily ← silver chat + meeting (m365 partitions)
-- ---------------------------------------------------------------------
-- Both inputs are filtered to data_source='insight_m365' in subqueries
-- before the FULL OUTER JOIN — keeps the join condition limited to keys.
DROP VIEW IF EXISTS insight.teams_person_daily;
CREATE VIEW insight.teams_person_daily AS
SELECT
    lower(coalesce(c.email, m.email))         AS person_id,
    coalesce(c.date, m.date)                  AS metric_date,
    toFloat64(coalesce(c.total_chat_messages, 0))
                                              AS teams_messages,
    toFloat64(coalesce(m.meetings_attended, 0))
                                              AS teams_meetings,
    toFloat64(coalesce(m.calls_count, 0))     AS teams_calls
FROM (
    SELECT email, date, total_chat_messages
    FROM silver.class_collab_chat_activity
    WHERE data_source = 'insight_m365'
) AS c
FULL OUTER JOIN (
    SELECT email, date, meetings_attended, calls_count
    FROM silver.class_collab_meeting_activity
    WHERE data_source = 'insight_m365'
) AS m
    ON  lower(c.email) = lower(m.email)
    AND c.date         = m.date;

-- ---------------------------------------------------------------------
-- files_person_daily ← silver.class_collab_document_activity
-- ---------------------------------------------------------------------
DROP VIEW IF EXISTS insight.files_person_daily;
CREATE VIEW insight.files_person_daily AS
SELECT
    lower(email)                              AS person_id,
    date                                      AS metric_date,
    toFloat64(sum(coalesce(shared_internally_count, 0)))
        + toFloat64(sum(coalesce(shared_externally_count, 0)))
                                              AS files_shared
FROM silver.class_collab_document_activity
WHERE data_source = 'insight_m365'
  AND email IS NOT NULL
  AND email != ''
GROUP BY lower(email), date;

-- ---------------------------------------------------------------------
-- comms_daily ← UNION of all four silver collab tables
-- ---------------------------------------------------------------------
DROP VIEW IF EXISTS insight.comms_daily;
CREATE VIEW insight.comms_daily AS
SELECT
    person_id,
    toString(metric_date)                     AS metric_date,
    sum(emails_sent)                          AS emails_sent,
    sum(zoom_calls)                           AS zoom_calls,
    sum(meeting_hours)                        AS meeting_hours,
    sum(teams_messages)                       AS teams_messages,
    sum(teams_meetings)                       AS teams_meetings,
    sum(files_shared)                         AS files_shared
FROM (
    SELECT
        lower(person_key)                     AS person_id,
        date                                  AS metric_date,
        toFloat64(coalesce(sent_count, 0))    AS emails_sent,
        toFloat64(0)                          AS zoom_calls,
        toFloat64(0)                          AS meeting_hours,
        toFloat64(0)                          AS teams_messages,
        toFloat64(0)                          AS teams_meetings,
        toFloat64(0)                          AS files_shared
    FROM silver.class_collab_email_activity
    WHERE data_source = 'insight_m365'

    UNION ALL

    SELECT
        lower(email),
        date,
        toFloat64(0),
        toFloat64(coalesce(calls_count, 0)),
        toFloat64(coalesce(audio_duration_seconds, 0)) / 3600.0,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0)
    FROM silver.class_collab_meeting_activity
    WHERE data_source = 'insight_zoom'

    UNION ALL

    SELECT
        lower(email),
        date,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(coalesce(total_chat_messages, 0)),
        toFloat64(0),
        toFloat64(0)
    FROM silver.class_collab_chat_activity
    WHERE data_source = 'insight_m365'

    UNION ALL

    SELECT
        lower(email),
        date,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(coalesce(meetings_attended, 0)),
        toFloat64(0)
    FROM silver.class_collab_meeting_activity
    WHERE data_source = 'insight_m365'

    UNION ALL

    SELECT
        lower(email),
        date,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(coalesce(shared_internally_count, 0))
            + toFloat64(coalesce(shared_externally_count, 0))
    FROM silver.class_collab_document_activity
    WHERE data_source = 'insight_m365'
) AS sub
WHERE person_id IS NOT NULL AND person_id != ''
GROUP BY person_id, metric_date;

-- ---------------------------------------------------------------------
-- jira_person_daily ← bronze_jira.jira_issue WITH argMax dedup
-- ---------------------------------------------------------------------
-- The previous definition referenced a non-existent column `issue_type`
-- (renamed/removed during a bronze schema migration), so the view was
-- broken on virtuozzo. silver.class_task_field_history would be the
-- right source long-term, but it stores per-field events (not per-issue
-- snapshots), so reproducing the gold contract needs a new
-- silver.class_task_daily — out of scope for this PR.
--
-- Until then, query bronze with argMax(... _airbyte_extracted_at) +
-- GROUP BY unique_key to fold the ×N Airbyte append duplicates into one
-- row per issue (same dedup pattern insight.people uses successfully).
DROP VIEW IF EXISTS insight.jira_person_daily;
CREATE VIEW insight.jira_person_daily AS
SELECT
    lower(JSONExtractString(latest_fields, 'assignee', 'emailAddress')) AS person_id,
    toDate(parseDateTimeBestEffortOrNull(latest_updated))               AS metric_date,
    JSONExtractString(latest_fields, 'issuetype', 'name')               AS issue_type,
    JSONExtractString(latest_fields, 'status', 'name')                  AS status_name,
    JSONExtractString(latest_fields, 'resolution', 'name')              AS resolution,
    latest_due_date                                                     AS due_date,
    JSONExtractFloat(latest_fields, 'timeoriginalestimate')             AS time_estimate_sec,
    JSONExtractFloat(latest_fields, 'timespent')                        AS time_spent_sec,
    latest_id_readable                                                  AS id_readable
FROM (
    SELECT
        unique_key,
        argMax(custom_fields_json, _airbyte_extracted_at)               AS latest_fields,
        argMax(updated, _airbyte_extracted_at)                          AS latest_updated,
        argMax(due_date, _airbyte_extracted_at)                         AS latest_due_date,
        argMax(id_readable, _airbyte_extracted_at)                      AS latest_id_readable
    FROM bronze_jira.jira_issue
    WHERE unique_key IS NOT NULL
    GROUP BY unique_key
)
WHERE JSONExtractString(latest_fields, 'assignee', 'emailAddress') != '';

-- ---------------------------------------------------------------------
-- collab_bullet_rows ← silver class_collab_chat_activity (slack section)
-- ---------------------------------------------------------------------
-- Replaces the three bronze_slack.users_details branches with reads of
-- silver.class_collab_chat_activity (data_source='insight_slack'):
--   messages_posted_count          → total_chat_messages
--   channel_messages_posted_count  → channel_posts
DROP VIEW IF EXISTS insight.collab_bullet_rows;
CREATE VIEW insight.collab_bullet_rows AS
SELECT
    lower(e.email)                                AS person_id,
    p.org_unit_id,
    toString(e.date)                              AS metric_date,
    'm365_emails_sent'                            AS metric_key,
    toFloat64(ifNull(e.sent_count, 0))            AS metric_value
FROM silver.class_collab_email_activity AS e
LEFT JOIN insight.people AS p ON lower(e.email) = p.person_id
WHERE e.data_source = 'insight_m365'

UNION ALL
SELECT
    lower(m.email), p.org_unit_id, toString(m.date), 'zoom_calls',
    toFloat64(ifNull(m.calls_count, 0))
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
WHERE c.data_source = 'insight_m365'

UNION ALL
SELECT
    lower(d.email), p.org_unit_id, toString(d.date), 'm365_files_shared',
    toFloat64(ifNull(d.shared_internally_count, 0)) +
    toFloat64(ifNull(d.shared_externally_count, 0))
FROM silver.class_collab_document_activity AS d
LEFT JOIN insight.people AS p ON lower(d.email) = p.person_id
WHERE d.data_source = 'insight_m365'

UNION ALL
SELECT
    f.email, p.org_unit_id, toString(f.day), 'meeting_free',
    if(ifNull(f.meeting_hours, 0) = 0, toFloat64(1), toFloat64(0))
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id

UNION ALL
SELECT
    lower(s.email), p.org_unit_id, toString(s.date), 'slack_thread_participation',
    toFloat64(ifNull(s.channel_posts, 0))
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
WHERE s.data_source = 'insight_slack'

UNION ALL
SELECT
    lower(s.email), p.org_unit_id, toString(s.date), 'slack_message_engagement',
    toFloat64(ifNull(s.total_chat_messages, 0))
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
WHERE s.data_source = 'insight_slack'

UNION ALL
SELECT
    lower(s.email), p.org_unit_id, toString(s.date), 'slack_dm_ratio',
    -- 0 messages → no rate to compute; return NULL not 0.
    if(ifNull(s.total_chat_messages, 0) > 0,
       round(((toFloat64(ifNull(s.total_chat_messages, 0)) -
               toFloat64(ifNull(s.channel_posts, 0))) /
              toFloat64(s.total_chat_messages)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
WHERE s.data_source = 'insight_slack';

-- ---------------------------------------------------------------------
-- ai_bullet_rows ← silver.class_ai_dev_usage
-- ---------------------------------------------------------------------
-- bronze_cursor.cursor_daily_usage maps cleanly to silver.class_ai_dev_usage:
--   isActive=true            → row exists (silver only ingests isActive=true)
--   acceptedLinesAdded       → silver.lines_added
--   totalLinesAdded          → silver.total_lines_added (added in this PR)
--   totalTabsShown           → silver.tool_use_offered
--   totalTabsAccepted        → silver.tool_use_accepted
--   agentRequests            → silver.agent_sessions
DROP VIEW IF EXISTS insight.ai_bullet_rows;
CREATE VIEW insight.ai_bullet_rows AS
SELECT
    lower(c.email)                                AS person_id,
    p.org_unit_id,
    c.day                                         AS metric_date,
    'active_ai_members'                           AS metric_key,
    toFloat64(1)                                  AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_active',
    toFloat64(1)
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_active',
    -- Claude Code Enterprise API not ingested.
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'codex_active',
    -- Codex not ingested.
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'team_ai_loc',
    toFloat64(coalesce(c.lines_added, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_acceptance',
    if(toFloat64(coalesce(c.tool_use_offered, 0)) > 0,
       round((toFloat64(coalesce(c.tool_use_accepted, 0)) /
              toFloat64(c.tool_use_offered)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_tool_acceptance',
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_completions',
    toFloat64(coalesce(c.completions_count, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_agents',
    toFloat64(coalesce(c.agent_sessions, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cursor_lines',
    toFloat64(coalesce(c.lines_added, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_sessions',
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_tool_accept',
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'cc_lines',
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'ai_loc_share2',
    if(toFloat64(coalesce(c.total_lines_added, 0)) > 0,
       round((toFloat64(coalesce(c.lines_added, 0)) /
              toFloat64(c.total_lines_added)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'claude_web',
    -- Claude.ai web not ingested.
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT
    lower(c.email), p.org_unit_id, c.day, 'chatgpt',
    -- ChatGPT Team not ingested.
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id;

-- ---------------------------------------------------------------------
-- exec_summary ← silver.class_ai_dev_usage (the cursor LEFT JOIN)
-- ---------------------------------------------------------------------
DROP VIEW IF EXISTS insight.exec_summary;
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
    base.org_unit_id                              AS org_unit_id,
    base.org_unit_name                            AS org_unit_name,
    org.headcount                                 AS headcount,
    ifNull(j.tasks_closed, 0)                     AS tasks_closed,
    ifNull(j.bugs_fixed, 0)                       AS bugs_fixed,
    CAST(NULL AS Nullable(Float64))               AS build_success_pct,
    greatest(0, least(100, round(base.avg_focus_pct, 1))) AS focus_time_pct,
    if(ai.active_count IS NULL,
       CAST(NULL AS Nullable(Float64)),
       round((ai.active_count * 100.) / greatest(org.headcount, 1), 1))
                                                  AS ai_adoption_pct,
    if(ai.avg_ai_loc_share IS NULL,
       CAST(NULL AS Nullable(Float64)),
       round(ai.avg_ai_loc_share, 1))             AS ai_loc_share_pct,
    CAST(NULL AS Nullable(Float64))               AS pr_cycle_time_h,
    base.metric_date                              AS metric_date
FROM (
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
INNER JOIN (
    SELECT org_unit_id, toUInt32(count()) AS headcount
    FROM insight.people
    WHERE status = 'Active'
    GROUP BY org_unit_id
) AS org ON base.org_unit_id = org.org_unit_id
LEFT JOIN (
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
LEFT JOIN (
    SELECT
        pe.org_unit_id,
        toString(c.day)               AS metric_date,
        countDistinct(lower(c.email)) AS active_count,
        avg(if(toFloat64(coalesce(c.total_lines_added, 0)) > 0,
               (toFloat64(coalesce(c.lines_added, 0)) /
                toFloat64(c.total_lines_added)) * 100,
               CAST(NULL AS Nullable(Float64)))) AS avg_ai_loc_share
    FROM silver.class_ai_dev_usage AS c
    INNER JOIN insight.people AS pe
        ON (lower(c.email) = pe.person_id) AND (pe.status = 'Active')
    GROUP BY pe.org_unit_id, c.day
) AS ai
    ON (base.org_unit_id = ai.org_unit_id) AND (base.metric_date = ai.metric_date);

-- ---------------------------------------------------------------------
-- team_member ← silver.class_ai_dev_usage (the cursor LEFT JOIN)
-- ---------------------------------------------------------------------
DROP VIEW IF EXISTS insight.team_member;
CREATE VIEW insight.team_member AS
SELECT
    p.person_id                                   AS person_id,
    p.display_name                                AS display_name,
    p.seniority                                   AS seniority,
    p.org_unit_id                                 AS org_unit_id,
    toFloat64(ifNull(j.tasks_closed, 0))          AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0))            AS bugs_fixed,
    if(f.dev_time_h IS NULL,
        CAST(NULL AS Nullable(Float64)),
        CAST(greatest(0, round(f.dev_time_h, 1)) AS Nullable(Float64)))
                                                  AS dev_time_h,
    CAST(NULL AS Nullable(Float64))               AS prs_merged,
    CAST(NULL AS Nullable(Float64))               AS build_success_pct,
    if(f.focus_time_pct IS NULL,
        CAST(NULL AS Nullable(Float64)),
        CAST(greatest(0, least(100, round(f.focus_time_pct, 1)))
             AS Nullable(Float64)))               AS focus_time_pct,
    if(cur.email IS NOT NULL,
       ['Cursor'],
       CAST([] AS Array(String)))                 AS ai_tools,
    if(cur.email IS NULL,
        CAST(NULL AS Nullable(Float64)),
        CAST(round(cur.ai_loc_share_pct, 1) AS Nullable(Float64)))
                                                  AS ai_loc_share_pct,
    f.metric_date                                 AS metric_date
FROM insight.people AS p
INNER JOIN (
    SELECT
        email,
        toString(day)             AS metric_date,
        focus_time_pct,
        dev_time_h
    FROM silver.class_focus_metrics
) AS f ON p.person_id = f.email
LEFT JOIN (
    SELECT
        person_id,
        toString(metric_date)     AS metric_date,
        sum(tasks_closed)         AS tasks_closed,
        sum(bugs_fixed)           AS bugs_fixed
    FROM insight.jira_closed_tasks
    GROUP BY person_id, metric_date
) AS j ON (p.person_id = j.person_id) AND (f.metric_date = j.metric_date)
LEFT JOIN (
    SELECT
        lower(email)              AS email,
        toString(day)             AS metric_date,
        if(toFloat64(coalesce(total_lines_added, 0)) > 0,
           round((toFloat64(coalesce(lines_added, 0)) /
                  toFloat64(total_lines_added)) * 100, 1),
           CAST(NULL AS Nullable(Float64))) AS ai_loc_share_pct
    FROM silver.class_ai_dev_usage
) AS cur ON (p.person_id = cur.email) AND (f.metric_date = cur.metric_date)
WHERE p.status = 'Active';

-- ---------------------------------------------------------------------
-- ic_kpis ← silver.class_ai_dev_usage (cursor) + silver.mtr_git_person_totals
-- ---------------------------------------------------------------------
-- loc / prs_merged / pr_cycle_time_h are person-level lifetime aggregates
-- carried from silver.mtr_git_person_totals (PR #198). Same value repeats
-- across each metric_date row for a given person — semantically the same
-- as the previous behavior (where these were Cursor-derived per-person
-- aggregates), now sourced from real git data instead.
DROP VIEW IF EXISTS insight.ic_kpis;
CREATE VIEW insight.ic_kpis AS
SELECT
    f.email                                       AS person_id,
    p.org_unit_id                                 AS org_unit_id,
    toString(f.day)                               AS metric_date,
    CAST(toFloat64(coalesce(t.loc, 0)) AS Nullable(Float64))
                                                  AS loc,
    round(ifNull(cur.ai_loc_share_pct, 0), 1)     AS ai_loc_share_pct,
    CAST(toFloat64(coalesce(t.prs_merged, 0)) AS Nullable(Float64))
                                                  AS prs_merged,
    if(t.avg_pr_cycle_time_h IS NULL,
       CAST(NULL AS Nullable(Float64)),
       CAST(round(t.avg_pr_cycle_time_h, 1) AS Nullable(Float64)))
                                                  AS pr_cycle_time_h,
    greatest(0, least(100, round(ifNull(f.focus_time_pct, 100), 1)))
                                                  AS focus_time_pct,
    toFloat64(ifNull(j.tasks_closed, 0))          AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0))            AS bugs_fixed,
    CAST(NULL, 'Nullable(Float64)')               AS build_success_pct,
    toFloat64(ifNull(cur.ai_sessions, 0))         AS ai_sessions
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id
LEFT JOIN silver.mtr_git_person_totals AS t
    ON t.person_key = f.email
LEFT JOIN (
    SELECT
        person_id,
        toString(metric_date)                     AS metric_date,
        sum(tasks_closed)                         AS tasks_closed,
        sum(bugs_fixed)                           AS bugs_fixed
    FROM insight.jira_closed_tasks
    GROUP BY person_id, metric_date
) AS j ON (f.email = j.person_id) AND (toString(f.day) = j.metric_date)
LEFT JOIN (
    SELECT
        lower(email)                              AS person_id,
        toString(day)                             AS metric_date,
        if(toFloat64(coalesce(total_lines_added, 0)) > 0,
           round((toFloat64(coalesce(lines_added, 0)) /
                  toFloat64(total_lines_added)) * 100, 1),
           CAST(NULL AS Nullable(Float64)))       AS ai_loc_share_pct,
        toFloat64(coalesce(agent_sessions, 0))
            + toFloat64(coalesce(chat_requests, 0))
                                                  AS ai_sessions
    FROM silver.class_ai_dev_usage
) AS cur ON (f.email = cur.person_id) AND (toString(f.day) = cur.metric_date);

-- ---------------------------------------------------------------------
-- ic_chart_loc ← silver.class_ai_dev_usage + silver.mtr_git_person_weekly
-- ---------------------------------------------------------------------
-- spec_lines now sourced from silver.mtr_git_person_weekly (PR #198) —
-- previously a hardcoded NULL placeholder. file_category='spec' is
-- classified at the fct_git_file_change layer.
DROP VIEW IF EXISTS insight.ic_chart_loc;
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
    lower(c.email)                                AS person_id,
    p.org_unit_id,
    toString(toStartOfWeek(toDate(c.day)))        AS date_bucket,
    toString(toStartOfWeek(toDate(c.day)))        AS metric_date,
    toFloat64(sum(coalesce(c.lines_added, 0)))    AS ai_loc,
    toFloat64(sum(coalesce(c.total_lines_added, 0))
              - sum(coalesce(c.lines_added, 0)))  AS code_loc,
    CAST(toFloat64(any(coalesce(g.spec_lines, 0))) AS Nullable(Float64))
                                                  AS spec_lines
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
LEFT JOIN silver.mtr_git_person_weekly AS g
    ON  g.person_key = lower(c.email)
    AND g.week       = toStartOfWeek(toDate(c.day))
GROUP BY
    lower(c.email),
    p.org_unit_id,
    toStartOfWeek(toDate(c.day));

-- ---------------------------------------------------------------------
-- ic_chart_delivery ← silver.mtr_git_person_weekly + insight.jira_closed_tasks
-- ---------------------------------------------------------------------
-- commits and prs_merged sourced directly from silver.mtr_git_person_weekly
-- (PR #198) — previously commits came via insight.commits_daily and
-- prs_merged was a NULL placeholder.
DROP VIEW IF EXISTS insight.ic_chart_delivery;
CREATE VIEW insight.ic_chart_delivery AS
WITH
    weekly_jira AS (
        SELECT
            person_id,
            toStartOfWeek(metric_date)            AS week,
            sum(tasks_closed)                     AS tasks_done
        FROM insight.jira_closed_tasks
        GROUP BY person_id, week
    ),
    weeks_all AS (
        SELECT person_key AS person_id, week
        FROM silver.mtr_git_person_weekly
        WHERE person_key != '' AND week IS NOT NULL
        UNION DISTINCT
        SELECT person_id, week FROM weekly_jira
    )
SELECT
    d.person_id                                   AS person_id,
    p.org_unit_id                                 AS org_unit_id,
    toString(d.week)                              AS date_bucket,
    toString(d.week)                              AS metric_date,
    toUInt64(ifNull(g.commits, 0))                AS commits,
    CAST(toUInt64(ifNull(g.prs_merged, 0)) AS Nullable(UInt64))
                                                  AS prs_merged,
    toUInt64(ifNull(j.tasks_done, 0))             AS tasks_done
FROM weeks_all                          AS d
LEFT JOIN insight.people                AS p ON d.person_id = p.person_id
LEFT JOIN silver.mtr_git_person_weekly  AS g
    ON g.person_key = d.person_id AND g.week = d.week
LEFT JOIN weekly_jira                   AS j
    ON j.person_id = d.person_id AND j.week = d.week;
