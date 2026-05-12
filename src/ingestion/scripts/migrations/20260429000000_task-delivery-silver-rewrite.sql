-- =====================================================================
-- Task Delivery — silver-only rewrite
-- =====================================================================
--
-- Architectural rule: gold reads silver only. Two task-delivery objects
-- still read bronze:
--   1. insight.jira_closed_tasks — MergeTree table populated by a
--      one-shot INSERT over bronze_jira.jira_issue
--      (20260422000000_gold-views.sql:102-135).
--   2. insight.task_delivery_bullet_rows — VIEW over jira_closed_tasks
--      (20260423120000_bullet-views-honest-nulls.sql:137-176).
--
-- This migration replaces both with silver-derived definitions and
-- expands the bullet from 5 to 9 metric_keys.
--
-- Silver inputs:
--   silver.class_task_field_history   FINAL (per-field event log)
--   silver.class_task_worklogs        FINAL (worklog rows)
--   silver.class_task_users           FINAL (account_id ↔ email)
--
-- Preserved metric_keys (5):
--   tasks_completed, task_dev_time, task_reopen_rate,
--   due_date_compliance, estimation_accuracy
--
-- New metric_keys (4):
--   worklog_logging_accuracy   logged seconds / in-progress seconds
--   bugs_to_task_ratio         bugs closed / tasks closed
--   mean_time_to_resolution    days from create to close
--   stale_in_progress          open issues idle >14 days
--
-- jira_closed_tasks keeps its column shape (person_id, metric_date,
-- tasks_closed, bugs_fixed, on_time_count, has_due_date_count,
-- avg_time_spent, avg_time_estimate) so unrelated downstream views
-- (insight.team_member, insight.exec_summary, insight.ic_kpis,
-- insight.ic_chart_delivery in 20260427120000_views-from-silver.sql,
-- 20260422100000_ic-kpis-honest-nulls.sql, 20260422150000_team-member-
-- honest-nulls.sql) keep working without further edits.
--
-- Rollout: paired with analytics-api migration
-- m20260429_000001_task_delivery_silver_rewrite which updates the
-- TEAM_BULLET_DELIVERY (UUID …03) and IC_BULLET_DELIVERY (UUID …11)
-- query_ref entries in MariaDB metrics. CH and Rust must apply
-- together; until both land, the new bullets render as ComingSoon.
--
-- NULL semantics: every metric emits CAST(NULL AS Nullable(Float64))
-- when the value is undefined (zero denominator, missing input). CH
-- avg() ignores NULLs, so honest nulls do not drag period averages.
--
-- Idempotent via DROP IF EXISTS; safe to re-run.
-- =====================================================================

-- ---------------------------------------------------------------------
-- Drop in reverse-dependency order
-- ---------------------------------------------------------------------
DROP VIEW  IF EXISTS insight.task_delivery_company_stats;
DROP VIEW  IF EXISTS insight.task_delivery_person_period;
DROP VIEW  IF EXISTS insight.task_delivery_bullet_rows;

-- helper views may not exist on first run; safe to drop unconditionally.
-- task_issue_current_state and task_status_intervals were originally VIEWs but
-- are referenced from 5+ downstream views — materialising them as MergeTree
-- tables cuts task_delivery bullet endpoint latency from ~1.8s to ~0.4s on
-- the local dump. Same trade-off the original `jira_closed_tasks` MergeTree
-- made: stale until the migration is re-run, but downstream reads are fast.
DROP VIEW  IF EXISTS insight.task_in_progress_seconds_per_day;
DROP VIEW  IF EXISTS insight.task_worklog_seconds_per_day;
DROP VIEW  IF EXISTS insight.task_reopen_in_window;
DROP VIEW  IF EXISTS insight.task_close_events_daily;
DROP VIEW  IF EXISTS insight.task_reopen_events_daily;
DROP VIEW  IF EXISTS insight.task_in_progress_seconds_per_close_day;
DROP VIEW  IF EXISTS insight.task_dev_seconds_per_issue;
DROP VIEW  IF EXISTS insight.task_status_intervals;
DROP TABLE IF EXISTS insight.task_status_intervals;
DROP VIEW  IF EXISTS insight.task_issue_current_state;
DROP TABLE IF EXISTS insight.task_issue_current_state;

-- jira_closed_tasks was a MergeTree TABLE; becoming a VIEW.
DROP TABLE IF EXISTS insight.jira_closed_tasks;

-- ---------------------------------------------------------------------
-- task_issue_current_state
-- ---------------------------------------------------------------------
-- One row per (insight_source_id, issue_id) with the latest scalar
-- field values needed by every downstream metric. Latest value per
-- field comes from argMax over class_task_field_history FINAL.
-- assignee_id is the account_id (value_id_type='account_id'); email is
-- joined from class_task_users. created_at is the earliest synthetic_
-- initial event time across any field for the issue. final_close_at is
-- the max event_at where status entered a closed state.
-- Refreshable MATERIALIZED VIEW (CH 24.4+). Backed by a hidden MergeTree
-- table; the SELECT below re-runs every hour and replaces the destination
-- atomically. Reads are fast (table scan), writes are off the request path.
-- Initial population is triggered explicitly via SYSTEM REFRESH VIEW at
-- the bottom of this migration so the view is non-empty after CREATE.
SET allow_experimental_refreshable_materialized_view = 1;
CREATE MATERIALIZED VIEW insight.task_issue_current_state
REFRESH EVERY 1 HOUR
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id)
SETTINGS index_granularity = 8192, allow_nullable_key = 1
AS
WITH issue_state AS (
    -- Single-pass per-field pivot via conditional argMax. The tie-breaker
    -- `(event_at, _version)` deterministically picks the latest write even
    -- when the source's ReplacingMergeTree merges haven't run yet, so we
    -- avoid `FINAL` on the read path. The "no duplicates per
    -- (issue, field, event_id)" invariant is enforced by
    -- `tests/task/assert_no_duplicate_silver_rows.sql`.
    --
    -- This replaces an earlier 9-CTE-with-FINAL implementation. Benchmark
    -- (CH 26.3, ~205k events on stage dump):
    --   * clean (1 part): old 52ms → new 28ms (~1.9x)
    --   * 4 unmerged parts: old 410ms → new 28ms (~15x)
    -- The old shape paid 9× table scans + 8 JOINs and degraded badly when
    -- the silver table had unmerged duplicates, which is the normal state
    -- during active ingestion.
    SELECT
        insight_source_id,
        data_source,
        issue_id,
        argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'status' AND delta_action = 'set')                    AS status_name,
        argMaxIf(value_ids[1], (event_at, _version),
                 field_id = 'assignee' AND delta_action = 'set')                  AS assignee_account_id,
        argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'issuetype' AND delta_action = 'set')                 AS issue_type,
        argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'priority' AND delta_action = 'set')                  AS priority,
        argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'duedate' AND delta_action = 'set')                   AS due_date_str,
        toFloat64OrNull(argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'timeoriginalestimate' AND delta_action = 'set'))     AS time_estimate_seconds,
        toFloat64OrNull(argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'timespent' AND delta_action = 'set'))                AS time_spent_seconds_field,
        minIf(event_at, event_kind = 'synthetic_initial')                         AS created_at,
        maxIf(event_at,
              field_id = 'status' AND delta_action = 'set'
              AND value_displays[1] IN ('Closed','Resolved','Verified'))          AS final_close_at,
        -- Most recent status change of any kind. Used by `stale_in_progress`
        -- to flag issues that haven't moved in N days. Hoisted into this
        -- view so consumers don't need a second scan of class_task_field_history.
        maxIf(event_at, field_id = 'status' AND delta_action = 'set')             AS last_status_event_at
    FROM silver.class_task_field_history
    WHERE field_id IN ('status','assignee','issuetype','priority','duedate',
                       'timeoriginalestimate','timespent')
       OR event_kind = 'synthetic_initial'
    GROUP BY insight_source_id, data_source, issue_id
)
SELECT
    s.insight_source_id                                              AS insight_source_id,
    s.data_source                                                    AS data_source,
    s.issue_id                                                       AS issue_id,
    s.status_name                                                    AS status_name,
    s.assignee_account_id                                            AS assignee_account_id,
    s.issue_type                                                     AS issue_type,
    s.priority                                                       AS priority,
    s.due_date_str                                                   AS due_date_str,
    s.time_estimate_seconds                                          AS time_estimate_seconds,
    s.time_spent_seconds_field                                       AS time_spent_seconds_field,
    s.created_at                                                     AS created_at,
    s.final_close_at                                                 AS final_close_at,
    s.last_status_event_at                                           AS last_status_event_at,
    lower(u.email)                                                   AS assignee_email,
    p.org_unit_id                                                    AS org_unit_id
FROM issue_state AS s
LEFT JOIN silver.class_task_users AS u FINAL
    ON  u.insight_source_id = s.insight_source_id
    AND u.user_id           = s.assignee_account_id
LEFT JOIN insight.people AS p ON p.person_id = lower(u.email);

-- ---------------------------------------------------------------------
-- task_status_intervals
-- ---------------------------------------------------------------------
-- Pair adjacent status events into (start, end) intervals. The last
-- interval is open: clamped to final_close_at for closed issues, now()
-- for still-open issues. Refreshable MV — same rationale as
-- task_issue_current_state above.
CREATE MATERIALIZED VIEW insight.task_status_intervals
REFRESH EVERY 1 HOUR
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id, interval_start)
SETTINGS index_granularity = 8192, allow_nullable_key = 1
AS
WITH events AS (
    SELECT
        insight_source_id,
        issue_id,
        arraySort(
            x -> x.1,
            groupArray((event_at, value_displays[1]))
        ) AS evs
    FROM silver.class_task_field_history FINAL
    WHERE field_id = 'status' AND delta_action = 'set'
    GROUP BY insight_source_id, issue_id
)
SELECT
    insight_source_id,
    issue_id,
    interval_start,
    interval_end,
    status_name,
    duration_seconds
FROM (
    SELECT
        e.insight_source_id                                      AS insight_source_id,
        e.issue_id                                               AS issue_id,
        arrayJoin(arrayMap(
            i -> (
                (e.evs[i]).1,
                if(i = length(e.evs),
                   ifNull(s.final_close_at, now()),
                   (e.evs[i + 1]).1),
                (e.evs[i]).2
            ),
            range(1, length(e.evs) + 1)
        )) AS row,
        row.1                                                    AS interval_start,
        row.2                                                    AS interval_end,
        row.3                                                    AS status_name,
        toFloat64(greatest(toInt64(0),
                           dateDiff('second', row.1, row.2)))    AS duration_seconds,
        s.created_at                                             AS issue_created_at
    FROM events AS e
    LEFT JOIN insight.task_issue_current_state AS s
        ON s.insight_source_id = e.insight_source_id AND s.issue_id = e.issue_id
)
-- Defensive: drop intervals with bogus timestamps. The dataset has had
-- two failure modes:
--   * epoch-0 (Unix 1970-01-01) close events from corrupted changelog rows
--     where the parsed timestamp came back zero;
--   * intervals starting before the issue's own creation, from out-of-order
--     synthetic_initial events.
-- Without this filter a single bad row blows up ARRAY JOIN with a
-- `range(billions)` length. We use the issue's `created_at` as a per-row
-- floor, falling back to the day after epoch for rows where created_at
-- itself is unknown — this is strictly tighter than the previous fixed
-- 2010-01-01 cutoff, which was an arbitrary magic number.
WHERE interval_start >= ifNull(issue_created_at, toDateTime('1970-01-02'))
  AND interval_end   >= interval_start
  AND interval_end   <= now() + INTERVAL 1 DAY;

-- ---------------------------------------------------------------------
-- task_dev_seconds_per_issue
-- ---------------------------------------------------------------------
-- One row per closed issue with three lifetime durations that drive the
-- per-task time bullets:
--   `dev_seconds`    — total time in dev-active statuses (drives task_dev_time)
--   `lead_seconds`   — total lifetime (created → final close), drives mean_time_to_resolution
--   `pickup_seconds` — created → first dev-active status entry, drives pickup_time
-- `flow_efficiency` is derived in the bullet view as dev_seconds / lead_seconds × 100.
-- All grouped per-task so the period-level median picks a typical task
-- instead of being skewed by a single year-old issue closed in-window.
CREATE VIEW insight.task_dev_seconds_per_issue AS
SELECT
    s.assignee_email                                             AS assignee_email,
    s.insight_source_id                                          AS insight_source_id,
    s.issue_id                                                   AS issue_id,
    toDate(s.final_close_at)                                     AS close_date,
    sum(i.duration_seconds)                                      AS dev_seconds,
    -- Lead time: created (synthetic_initial event_at min, surfaced as
    -- task_issue_current_state.created_at) → final_close_at. Clamped at 0
    -- so out-of-order changelog data (created_at > final_close_at) doesn't
    -- emit negative durations into mean_time_to_resolution downstream.
    if(any(s.created_at) IS NULL,
       CAST(NULL AS Nullable(Float64)),
       toFloat64(greatest(toInt64(0),
                          dateDiff('second', any(s.created_at), any(s.final_close_at)))))
                                                                 AS lead_seconds,
    -- Pickup time: from creation to the first dev-active interval start
    -- (i.interval_start is already filtered to dev statuses by the JOIN).
    if(any(s.created_at) IS NULL OR min(i.interval_start) IS NULL,
       CAST(NULL AS Nullable(Float64)),
       toFloat64(greatest(toInt64(0),
                          dateDiff('second', any(s.created_at), min(i.interval_start)))))
                                                                 AS pickup_seconds
FROM insight.task_issue_current_state AS s
LEFT JOIN insight.task_status_intervals AS i
    ON  i.insight_source_id = s.insight_source_id
    AND i.issue_id          = s.issue_id
    AND i.status_name       IN (
        'In Progress',
        'In Development', 'In  Development',  -- second is a workflow typo seen in prod data
        'Code Review', 'In Review', 'In PM Review',
        'In QA', 'In Design',
        'Waiting for Merge'
    )
WHERE s.final_close_at IS NOT NULL
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
GROUP BY s.assignee_email, s.insight_source_id, s.issue_id, close_date;

-- ---------------------------------------------------------------------
-- task_close_events_daily / task_reopen_events_daily
-- ---------------------------------------------------------------------
-- Two parallel event streams that drive `task_reopen_rate` as a ratio of
-- period-aligned sums (period = dashboard date filter). Each row is a +1
-- "close happened" or +1 "reopen happened" tagged with its event date and
-- the issue's current assignee. The bullet view encodes them with opposite
-- signs into the same metric_key so a single OData filter on metric_date
-- naturally scopes both numerator and denominator to the chosen period.
--
-- Both streams count *every* status transition (not just the final close):
-- a close→reopen→close cycle contributes 2 closes and 1 reopen. Counting
-- only `final_close_at` would have produced 1 close / 1 reopen and made
-- the rate read 100% for what is structurally a 50% rebound.
-- Both views derive transitions from `task_status_intervals`. Each
-- interval row already represents one status change at `interval_start`,
-- pre-deduplicated by the source MV's FINAL read. We use a window
-- function to look at the previous interval's status — same shape as
-- the earlier silver-direct version, but reads ~10× less data and avoids
-- a redundant FINAL scan over class_task_field_history.
CREATE VIEW insight.task_close_events_daily AS
WITH transitions AS (
    SELECT
        insight_source_id,
        issue_id,
        interval_start AS event_at,
        status_name,
        lagInFrame(status_name) OVER (
            PARTITION BY insight_source_id, issue_id
            ORDER BY interval_start
        ) AS prev_status
    FROM insight.task_status_intervals
)
SELECT
    s.assignee_email                                             AS assignee_email,
    toDate(t.event_at)                                           AS event_date,
    count()                                                      AS close_count
FROM transitions AS t
INNER JOIN insight.task_issue_current_state AS s
    ON  s.insight_source_id = t.insight_source_id
    AND s.issue_id          = t.issue_id
WHERE (t.prev_status IS NULL OR t.prev_status NOT IN ('Closed','Resolved','Verified'))
  AND t.status_name IN ('Closed','Resolved','Verified')
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
GROUP BY assignee_email, event_date;

CREATE VIEW insight.task_reopen_events_daily AS
WITH transitions AS (
    SELECT
        insight_source_id,
        issue_id,
        interval_start AS event_at,
        status_name,
        lagInFrame(status_name) OVER (
            PARTITION BY insight_source_id, issue_id
            ORDER BY interval_start
        ) AS prev_status
    FROM insight.task_status_intervals
)
SELECT
    s.assignee_email                                             AS assignee_email,
    toDate(t.event_at)                                           AS event_date,
    count()                                                      AS reopen_count
FROM transitions AS t
INNER JOIN insight.task_issue_current_state AS s
    ON  s.insight_source_id = t.insight_source_id
    AND s.issue_id          = t.issue_id
WHERE t.prev_status IN ('Closed','Resolved','Verified')
  AND t.status_name NOT IN ('Closed','Resolved','Verified')
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
GROUP BY assignee_email, event_date;

-- ---------------------------------------------------------------------
-- task_worklog_seconds_per_day
-- ---------------------------------------------------------------------
-- Per (author_email, work_date): sum of worklog duration_seconds.
-- Drives `worklog_logging_accuracy` numerator.
CREATE VIEW insight.task_worklog_seconds_per_day AS
SELECT
    lower(u.email)                                               AS author_email,
    toDate(w.work_date)                                          AS work_date,
    sum(ifNull(w.duration_seconds, toFloat64(0)))                AS worklog_seconds
FROM silver.class_task_worklogs AS w FINAL
INNER JOIN silver.class_task_users AS u FINAL
    ON  u.insight_source_id = w.insight_source_id
    AND u.user_id           = w.author_id
WHERE u.email IS NOT NULL AND u.email != ''
GROUP BY author_email, work_date;

-- ---------------------------------------------------------------------
-- task_in_progress_seconds_per_day
-- ---------------------------------------------------------------------
-- Per (assignee_email, day): total seconds spent in 'In Progress'
-- whose interval intersects that day. Day overlap = max(0, min(end,
-- day_end) - max(start, day_start)). Drives `worklog_logging_
-- accuracy` denominator.
CREATE VIEW insight.task_in_progress_seconds_per_day AS
WITH ip AS (
    SELECT
        s.assignee_email                                         AS assignee_email,
        i.interval_start                                         AS interval_start,
        i.interval_end                                           AS interval_end
    FROM insight.task_status_intervals AS i
    INNER JOIN insight.task_issue_current_state AS s
        ON s.insight_source_id = i.insight_source_id AND s.issue_id = i.issue_id
    WHERE i.status_name IN (
        'In Progress',
        'In Development', 'In  Development',
        'Code Review', 'In Review', 'In PM Review',
        'In QA', 'In Design',
        'Waiting for Merge'
    )
      AND s.assignee_email IS NOT NULL
      AND s.assignee_email != ''
)
SELECT
    assignee_email,
    day,
    sum(toFloat64(greatest(
        toInt64(0),
        dateDiff('second',
                 greatest(interval_start, toDateTime(day)),
                 least(interval_end, toDateTime(day) + toIntervalDay(1)))
    ))) AS in_progress_seconds
FROM ip
ARRAY JOIN
    arrayMap(d -> toDate(interval_start) + toIntervalDay(d),
             range(toUInt32(dateDiff('day',
                                     toDate(interval_start),
                                     toDate(interval_end)) + 1))) AS day
GROUP BY assignee_email, day;

-- =====================================================================
-- jira_closed_tasks — silver-derived VIEW, same column shape as before
-- =====================================================================
-- Aggregates per (assignee_email, close_date). avg_time_spent and
-- avg_time_estimate read the Jira-tracked timespent / timeoriginal-
-- estimate scalar fields (latest value per issue) — used by
-- estimation_accuracy. bugs_fixed counts issues where issue_type='Bug'.
-- Column types deliberately inferred from SELECT (no explicit list) — silver
-- assignee_email is Nullable(String); pinning person_id to non-Nullable String
-- triggers analyzer-level CAST errors before the WHERE filter applies.
-- Downstream consumers (team_member, ic_kpis, ic_chart_delivery) already
-- defensive-wrap with ifNull/coalesce.
CREATE VIEW insight.jira_closed_tasks AS
SELECT
    coalesce(s.assignee_email, '')                               AS person_id,
    toDate(s.final_close_at)                                     AS metric_date,
    toUInt64(count())                                            AS tasks_closed,
    toUInt64(countIf(s.issue_type = 'Bug'))                      AS bugs_fixed,
    toUInt64(countIf(
        s.due_date_str IS NOT NULL AND s.due_date_str != ''
        AND toDate(s.final_close_at) <= toDate(parseDateTimeBestEffortOrNull(s.due_date_str))
    ))                                                           AS on_time_count,
    toUInt64(countIf(s.due_date_str IS NOT NULL AND s.due_date_str != '')) AS has_due_date_count,
    avgIf(s.time_spent_seconds_field,
          ifNull(s.time_estimate_seconds, toFloat64(0)) > 0)     AS avg_time_spent,
    avgIf(s.time_estimate_seconds,
          ifNull(s.time_estimate_seconds, toFloat64(0)) > 0)     AS avg_time_estimate
FROM insight.task_issue_current_state AS s
WHERE s.final_close_at IS NOT NULL
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
  AND s.status_name IN ('Closed','Resolved','Verified')
GROUP BY person_id, metric_date;

-- =====================================================================
-- task_delivery_bullet_rows — 9 metric_keys
-- =====================================================================
CREATE VIEW insight.task_delivery_bullet_rows AS

-- 1. tasks_completed: count of issues closed that day per assignee.
SELECT
    j.person_id                                                  AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    toString(j.metric_date)                                      AS metric_date,
    'tasks_completed'                                            AS metric_key,
    CAST(toFloat64(j.tasks_closed) AS Nullable(Float64))         AS metric_value
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id

-- 2. task_dev_time: per-task hours in dev statuses (In Progress, Review,
--    QA, …) — one row per closed issue. query_ref takes the period-level
--    median (MEDIAN_LIST), so a single year-old issue closed in window
--    doesn't drag the team value up.
UNION ALL
SELECT
    ip.assignee_email,
    p.org_unit_id,
    toString(ip.close_date),
    'task_dev_time',
    if(ip.dev_seconds IS NULL OR ip.dev_seconds = 0,
       CAST(NULL AS Nullable(Float64)),
       round(toFloat64(ip.dev_seconds) / 3600.0, 2))
FROM insight.task_dev_seconds_per_issue AS ip
LEFT JOIN insight.people AS p ON ip.assignee_email = p.person_id

-- 3. task_reopen_rate: ratio-of-sums over the dashboard period. Two
--    branches under one metric_key, distinguished by sign of metric_value:
--      +1 per close event (positive)
--      -1 per reopen event (negative)
--    `query_ref` aggregates these with sumIf(>0) and -sumIf(<0) and
--    computes the rate. OData `metric_date` filter naturally scopes both
--    numerator and denominator to the same window.
UNION ALL
SELECT
    c.assignee_email,
    p.org_unit_id,
    toString(c.event_date),
    'task_reopen_rate',
    toFloat64(c.close_count)                                     AS metric_value
FROM insight.task_close_events_daily AS c
LEFT JOIN insight.people AS p ON c.assignee_email = p.person_id

UNION ALL
SELECT
    r.assignee_email,
    p.org_unit_id,
    toString(r.event_date),
    'task_reopen_rate',
    -toFloat64(r.reopen_count)                                   AS metric_value
FROM insight.task_reopen_events_daily AS r
LEFT JOIN insight.people AS p ON r.assignee_email = p.person_id

-- 4. due_date_compliance: of closed-that-day issues with a due date,
--    fraction closed on or before due_date.
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'due_date_compliance',
    if(j.has_due_date_count > 0,
       round((toFloat64(j.on_time_count) / toFloat64(j.has_due_date_count)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id

-- 5. estimation_accuracy: raw (estimate/spent)*100 ratio. Symmetric
--    folding around 100 happens at the period-aggregation layer
--    (analytics-api query_ref). NULL when either side is zero/missing.
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'estimation_accuracy',
    if(ifNull(j.avg_time_spent, toFloat64(0)) > 0
       AND j.avg_time_estimate IS NOT NULL,
       round((j.avg_time_estimate / j.avg_time_spent) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id

-- 6. worklog_logging_accuracy: per (person, day) ratio of
--    sum(worklog seconds logged that day) /
--    sum(in-progress seconds that day) * 100. NULL when denominator
--    is zero. Daily ratio can exceed 100% (worklog covers prior
--    in-progress time logged late) — folded at period layer same as
--    estimation_accuracy.
UNION ALL
SELECT
    coalesce(w.author_email, ip.assignee_email)                  AS person_id,
    p.org_unit_id,
    toString(coalesce(w.work_date, ip.day))                      AS metric_date,
    'worklog_logging_accuracy',
    if(ifNull(ip.in_progress_seconds, toFloat64(0)) > 0,
       round((toFloat64(ifNull(w.worklog_seconds, toFloat64(0))) /
              toFloat64(ip.in_progress_seconds)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM insight.task_worklog_seconds_per_day AS w
FULL OUTER JOIN insight.task_in_progress_seconds_per_day AS ip
    ON w.author_email = ip.assignee_email AND w.work_date = ip.day
LEFT JOIN insight.people AS p
    ON p.person_id = coalesce(w.author_email, ip.assignee_email)

-- 7. bugs_to_task_ratio: per (person, day) bugs/tasks * 100.
--    NULL when no tasks closed that day.
UNION ALL
SELECT
    j.person_id,
    p.org_unit_id,
    toString(j.metric_date),
    'bugs_to_task_ratio',
    if(j.tasks_closed > 0,
       round((toFloat64(j.bugs_fixed) / toFloat64(j.tasks_closed)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id

-- 8. mean_time_to_resolution: per-task lifetime (created → final close)
--    in days. One row per closed issue; query_ref takes period median.
UNION ALL
SELECT
    ip.assignee_email,
    p.org_unit_id,
    toString(ip.close_date),
    'mean_time_to_resolution',
    if(ip.lead_seconds IS NULL OR ip.lead_seconds = 0,
       CAST(NULL AS Nullable(Float64)),
       round(toFloat64(ip.lead_seconds) / 86400.0, 2))
FROM insight.task_dev_seconds_per_issue AS ip
LEFT JOIN insight.people AS p ON ip.assignee_email = p.person_id

-- 8b. flow_efficiency: per-task ratio of dev-status time to total
--     lifetime, %. NULL when lead = 0 (same-day close) or no dev time
--     recorded. query_ref takes period median.
UNION ALL
SELECT
    ip.assignee_email,
    p.org_unit_id,
    toString(ip.close_date),
    'flow_efficiency',
    if(ip.lead_seconds IS NULL OR ip.lead_seconds <= 0
       OR ip.dev_seconds IS NULL OR ip.dev_seconds = 0,
       CAST(NULL AS Nullable(Float64)),
       round(least(toFloat64(100),
                   (toFloat64(ip.dev_seconds) / toFloat64(ip.lead_seconds)) * 100), 1))
FROM insight.task_dev_seconds_per_issue AS ip
LEFT JOIN insight.people AS p ON ip.assignee_email = p.person_id

-- 8c. pickup_time: per-task days from creation to first dev-status
--     entry — measures queue time before active work starts.
UNION ALL
SELECT
    ip.assignee_email,
    p.org_unit_id,
    toString(ip.close_date),
    'pickup_time',
    if(ip.pickup_seconds IS NULL,
       CAST(NULL AS Nullable(Float64)),
       round(toFloat64(ip.pickup_seconds) / 86400.0, 2))
FROM insight.task_dev_seconds_per_issue AS ip
LEFT JOIN insight.people AS p ON ip.assignee_email = p.person_id

-- 9. stale_in_progress: count of currently-open issues per assignee
--    whose last status event is >14 days before today. Emitted only
--    once (today's date) since "currently open" is a snapshot.
UNION ALL
SELECT
    s.assignee_email                                             AS person_id,
    p.org_unit_id,
    toString(today())                                            AS metric_date,
    'stale_in_progress',
    CAST(toFloat64(count()) AS Nullable(Float64))                AS metric_value
FROM insight.task_issue_current_state AS s
LEFT JOIN insight.people AS p ON s.assignee_email = p.person_id
WHERE (s.status_name IS NULL OR s.status_name NOT IN ('Closed','Resolved','Verified'))
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
  AND s.last_status_event_at IS NOT NULL
  AND dateDiff('day', s.last_status_event_at, now()) > 14
GROUP BY s.assignee_email, p.org_unit_id;

-- =====================================================================
-- The pre-rewrite versions of `task_delivery_person_period` and
-- `task_delivery_company_stats` (defined in 20260422000000_gold-views.sql
-- and 20260423120000_bullet-views-honest-nulls.sql) had no consumer in
-- the analytics-api or frontend — the bullet endpoints query
-- `task_delivery_bullet_rows` directly through the per-metric reducers
-- in MariaDB `metrics.query_ref`. Recreating them here would require
-- duplicating the (now richer) reducer logic across two places. The
-- DROPs above remove them; if a future consumer needs an aggregated
-- per-period summary, build one against `task_delivery_bullet_rows`
-- using the same reducer structure as the analytics-api migration.
-- =====================================================================

-- =====================================================================
-- Initial population of the refreshable MVs is intentionally NOT done
-- here. `SYSTEM REFRESH VIEW` is synchronous and would block the
-- migration runner until both populates finish; on large tenants that
-- can mean the deploy hangs (or hits a timeout) waiting on the
-- recompute. The two views are designed to populate on their own
-- 1-hour tick after creation; downstream consumers
-- (`task_dev_seconds_per_issue`, `task_close_events_daily`,
-- `task_delivery_bullet_rows`) gracefully render empty (ComingSoon)
-- until then.
--
-- If you need to populate immediately after deploy without waiting for
-- the tick, run `scripts/post-deploy/refresh-task-views.sh` against the
-- target ClickHouse — that script issues the same two SYSTEM REFRESH
-- VIEW statements outside the migration path so a long recompute
-- doesn't block schema evolution.
-- =====================================================================
