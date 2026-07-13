-- =====================================================================
-- Task Delivery — close-detection via unified status_category (issue #1541)
-- =====================================================================
--
-- Supersedes the hardcoded status-display-name detection introduced in
-- 20260429000000_task-delivery-silver-rewrite.sql and the stale-branch of
-- 20260515000000_task-delivery-bullet-rewrite.sql. Those matched the status
-- *display name* against a literal list ('Closed','Resolved','Verified'), so a
-- default Jira Cloud "Done" or any non-English / custom workflow reported ZERO
-- closed tasks and blanked the whole Task Delivery panel.
--
-- This migration recreates the same objects (identical names + column shapes,
-- so downstream views — team_member, ic_kpis, exec_summary, ic_chart_delivery —
-- keep working unchanged) but derives lifecycle from the source-neutral
-- `silver.class_task_statuses.status_category`:
--     done        → task is closed        (was: name IN ('Closed','Resolved','Verified'))
--     in_progress → dev-active status     (was: name IN ('In Progress','In Review',...))
-- The join key is `class_task_field_history.value_ids[1]` (the status id) ↔
-- `class_task_statuses.status_id`. Jira maps statusCategory→status_category,
-- YouTrack maps isResolved→status_category; gold is now source-agnostic.
--
-- `task_worklog_seconds_per_day` is unchanged (no closedness dependency) and is
-- intentionally NOT recreated here.
--
-- Idempotent via DROP IF EXISTS; safe to re-run. Refreshable MVs are populated
-- on their own hourly tick (or via scripts/post-deploy/refresh-task-views.sh).
-- =====================================================================

-- ---------------------------------------------------------------------
-- Drop in reverse-dependency order
-- ---------------------------------------------------------------------
DROP VIEW  IF EXISTS insight.task_delivery_bullet_rows;
DROP VIEW  IF EXISTS insight.task_in_progress_seconds_per_day;
DROP VIEW  IF EXISTS insight.task_reopen_events_daily;
DROP VIEW  IF EXISTS insight.task_close_events_daily;
DROP VIEW  IF EXISTS insight.task_dev_seconds_per_issue;
DROP VIEW  IF EXISTS insight.jira_closed_tasks;
DROP VIEW  IF EXISTS insight.task_status_intervals;
DROP TABLE IF EXISTS insight.task_status_intervals;
DROP VIEW  IF EXISTS insight.task_issue_current_state;
DROP TABLE IF EXISTS insight.task_issue_current_state;

-- ---------------------------------------------------------------------
-- task_issue_current_state
-- ---------------------------------------------------------------------
-- Adds `status_id` (from value_ids[1]) and `status_category` (current status's
-- reconciled lifecycle), and derives `final_close_at` from the last transition
-- into a `done`-category status — no display-name literals.
SET allow_experimental_refreshable_materialized_view = 1;
CREATE MATERIALIZED VIEW insight.task_issue_current_state
REFRESH EVERY 1 HOUR
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id)
SETTINGS index_granularity = 8192, allow_nullable_key = 1
AS
WITH issue_state AS (
    -- Single-pass per-field pivot (unchanged from the predecessor) — plus the
    -- current status id, needed to join the status dimension below.
    SELECT
        insight_source_id,
        data_source,
        issue_id,
        argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'status' AND delta_action = 'set')                    AS status_name,
        argMaxIf(value_ids[1], (event_at, _version),
                 field_id = 'status' AND delta_action = 'set')                    AS status_id,
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
        maxIf(event_at, field_id = 'status' AND delta_action = 'set')             AS last_status_event_at
    FROM silver.class_task_field_history
    WHERE field_id IN ('status','assignee','issuetype','priority','duedate',
                       'timeoriginalestimate','timespent')
       OR event_kind = 'synthetic_initial'
    GROUP BY insight_source_id, data_source, issue_id
),
-- Per-issue close signal derived from the status dimension: the max event_at
-- where the status entered a `done` category. Requires the per-event category,
-- so this pass joins class_task_statuses BEFORE aggregating (unlike the scalar
-- pivot above, which needs no join).
status_cat AS (
    SELECT
        fh.insight_source_id                                                      AS insight_source_id,
        fh.issue_id                                                               AS issue_id,
        maxIf(fh.event_at, st.status_category = 'done')                           AS final_close_at
    FROM silver.class_task_field_history AS fh
    LEFT JOIN silver.class_task_statuses AS st FINAL
        ON  st.insight_source_id = fh.insight_source_id
        AND st.status_id         = fh.value_ids[1]
    WHERE fh.field_id = 'status' AND fh.delta_action = 'set'
    GROUP BY fh.insight_source_id, fh.issue_id
)
SELECT
    s.insight_source_id                                              AS insight_source_id,
    s.data_source                                                    AS data_source,
    s.issue_id                                                       AS issue_id,
    s.status_name                                                    AS status_name,
    s.status_id                                                      AS status_id,
    cur.status_category                                              AS status_category,
    s.assignee_account_id                                            AS assignee_account_id,
    s.issue_type                                                     AS issue_type,
    s.priority                                                       AS priority,
    s.due_date_str                                                   AS due_date_str,
    s.time_estimate_seconds                                          AS time_estimate_seconds,
    s.time_spent_seconds_field                                       AS time_spent_seconds_field,
    s.created_at                                                     AS created_at,
    sc.final_close_at                                                AS final_close_at,
    s.last_status_event_at                                           AS last_status_event_at,
    lower(u.email)                                                   AS assignee_email,
    p.org_unit_id                                                    AS org_unit_id
FROM issue_state AS s
LEFT JOIN status_cat AS sc
    ON sc.insight_source_id = s.insight_source_id AND sc.issue_id = s.issue_id
-- current status's category (for stale_in_progress / open-issue filters)
LEFT JOIN silver.class_task_statuses AS cur FINAL
    ON cur.insight_source_id = s.insight_source_id AND cur.status_id = s.status_id
LEFT JOIN silver.class_task_users AS u FINAL
    ON  u.insight_source_id = s.insight_source_id
    AND u.user_id           = s.assignee_account_id
LEFT JOIN insight.people AS p ON p.person_id = lower(u.email);

-- ---------------------------------------------------------------------
-- task_status_intervals
-- ---------------------------------------------------------------------
-- Carries the status id + reconciled status_category per interval so all
-- downstream span logic filters on category, not display name.
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
            groupArray((event_at, value_ids[1], value_displays[1]))
        ) AS evs
    FROM silver.class_task_field_history FINAL
    WHERE field_id = 'status' AND delta_action = 'set'
    GROUP BY insight_source_id, issue_id
)
SELECT
    iv.insight_source_id                                     AS insight_source_id,
    iv.issue_id                                              AS issue_id,
    iv.interval_start                                        AS interval_start,
    iv.interval_end                                          AS interval_end,
    iv.status_id                                             AS status_id,
    iv.status_name                                           AS status_name,
    st.status_category                                       AS status_category,
    iv.duration_seconds                                      AS duration_seconds
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
                (e.evs[i]).2,
                (e.evs[i]).3
            ),
            range(1, length(e.evs) + 1)
        )) AS row,
        row.1                                                    AS interval_start,
        row.2                                                    AS interval_end,
        row.3                                                    AS status_id,
        row.4                                                    AS status_name,
        toFloat64(greatest(toInt64(0),
                           dateDiff('second', row.1, row.2)))    AS duration_seconds,
        s.created_at                                             AS issue_created_at
    FROM events AS e
    LEFT JOIN insight.task_issue_current_state AS s
        ON s.insight_source_id = e.insight_source_id AND s.issue_id = e.issue_id
) AS iv
LEFT JOIN silver.class_task_statuses AS st FINAL
    ON st.insight_source_id = iv.insight_source_id AND st.status_id = iv.status_id
-- Same defensive interval-validity filter as the predecessor.
WHERE iv.interval_start >= ifNull(iv.issue_created_at, toDateTime('1970-01-02'))
  AND iv.interval_end   >= iv.interval_start
  AND iv.interval_end   <= now() + INTERVAL 1 DAY;

-- ---------------------------------------------------------------------
-- task_dev_seconds_per_issue
-- ---------------------------------------------------------------------
-- dev / lead / pickup per closed issue. Dev-active time is now every interval
-- whose status_category = 'in_progress' (was: hardcoded 'In Progress'/… list).
CREATE VIEW insight.task_dev_seconds_per_issue AS
SELECT
    s.assignee_email                                             AS assignee_email,
    s.insight_source_id                                          AS insight_source_id,
    s.issue_id                                                   AS issue_id,
    toDate(s.final_close_at)                                     AS close_date,
    sum(i.duration_seconds)                                      AS dev_seconds,
    if(any(s.created_at) IS NULL,
       CAST(NULL AS Nullable(Float64)),
       toFloat64(greatest(toInt64(0),
                          dateDiff('second', any(s.created_at), any(s.final_close_at)))))
                                                                 AS lead_seconds,
    if(any(s.created_at) IS NULL OR min(i.interval_start) IS NULL,
       CAST(NULL AS Nullable(Float64)),
       toFloat64(greatest(toInt64(0),
                          dateDiff('second', any(s.created_at), min(i.interval_start)))))
                                                                 AS pickup_seconds
FROM insight.task_issue_current_state AS s
LEFT JOIN insight.task_status_intervals AS i
    ON  i.insight_source_id = s.insight_source_id
    AND i.issue_id          = s.issue_id
    AND i.status_category   = 'in_progress'
WHERE s.final_close_at IS NOT NULL
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
GROUP BY s.assignee_email, s.insight_source_id, s.issue_id, close_date;

-- ---------------------------------------------------------------------
-- task_close_events_daily / task_reopen_events_daily
-- ---------------------------------------------------------------------
-- Close = transition INTO a done-category status; reopen = transition OUT of it.
-- Detected on status_category via lagInFrame, not display names.
CREATE VIEW insight.task_close_events_daily AS
WITH transitions AS (
    SELECT
        insight_source_id,
        issue_id,
        interval_start AS event_at,
        status_category,
        lagInFrame(status_category) OVER (
            PARTITION BY insight_source_id, issue_id
            ORDER BY interval_start
        ) AS prev_category
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
WHERE (t.prev_category IS NULL OR t.prev_category != 'done')
  AND t.status_category = 'done'
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
GROUP BY assignee_email, event_date;

CREATE VIEW insight.task_reopen_events_daily AS
WITH transitions AS (
    SELECT
        insight_source_id,
        issue_id,
        interval_start AS event_at,
        status_category,
        lagInFrame(status_category) OVER (
            PARTITION BY insight_source_id, issue_id
            ORDER BY interval_start
        ) AS prev_category
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
WHERE t.prev_category = 'done'
  AND (t.status_category != 'done' OR t.status_category IS NULL)
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
GROUP BY assignee_email, event_date;

-- ---------------------------------------------------------------------
-- task_in_progress_seconds_per_day
-- ---------------------------------------------------------------------
-- Per (assignee, day) seconds in an in_progress-category status (was: dev-name list).
CREATE VIEW insight.task_in_progress_seconds_per_day AS
WITH ip AS (
    SELECT
        s.assignee_email                                         AS assignee_email,
        i.interval_start                                         AS interval_start,
        i.interval_end                                           AS interval_end
    FROM insight.task_status_intervals AS i
    INNER JOIN insight.task_issue_current_state AS s
        ON s.insight_source_id = i.insight_source_id AND s.issue_id = i.issue_id
    WHERE i.status_category = 'in_progress'
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
-- jira_closed_tasks — same column shape; closed = status_category='done'
-- =====================================================================
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
  AND s.status_category = 'done'
GROUP BY person_id, metric_date;

-- =====================================================================
-- task_delivery_bullet_rows — recreated from 20260515 with the stale branch
-- filtering on status_category instead of the display-name list.
-- (All non-stale branches are byte-identical to 20260515.)
-- =====================================================================
CREATE VIEW insight.task_delivery_bullet_rows AS

SELECT
    j.person_id                                                  AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    j.metric_date                                                AS metric_date,
    kv.1                                                         AS metric_key,
    kv.2                                                         AS metric_value
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
ARRAY JOIN [
    ('tasks_completed',
        CAST(toFloat64(j.tasks_closed) AS Nullable(Float64))),
    ('due_date_on_time',
        CAST(toFloat64(j.on_time_count) AS Nullable(Float64))),
    ('due_date_with_due',
        CAST(toFloat64(j.has_due_date_count) AS Nullable(Float64))),
    ('estimation_accuracy',
        if(ifNull(j.avg_time_spent, toFloat64(0)) > 0
           AND j.avg_time_estimate IS NOT NULL,
           CAST(round((j.avg_time_estimate / j.avg_time_spent) * 100, 1)
                AS Nullable(Float64)),
           CAST(NULL AS Nullable(Float64)))),
    ('bugs_fixed',
        CAST(toFloat64(j.bugs_fixed) AS Nullable(Float64)))
] AS kv

UNION ALL

SELECT
    ip.assignee_email                                            AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    ip.close_date                                                AS metric_date,
    kv.1                                                         AS metric_key,
    kv.2                                                         AS metric_value
FROM insight.task_dev_seconds_per_issue AS ip
LEFT JOIN insight.people AS p ON ip.assignee_email = p.person_id
ARRAY JOIN [
    ('task_dev_time',
        if(ip.dev_seconds IS NULL OR ip.dev_seconds = 0,
           CAST(NULL AS Nullable(Float64)),
           CAST(round(toFloat64(ip.dev_seconds) / 3600.0, 2)
                AS Nullable(Float64)))),
    ('mean_time_to_resolution',
        if(ip.lead_seconds IS NULL OR ip.lead_seconds = 0,
           CAST(NULL AS Nullable(Float64)),
           CAST(round(toFloat64(ip.lead_seconds) / 86400.0, 2)
                AS Nullable(Float64)))),
    ('flow_efficiency_num',
        if(ip.dev_seconds IS NULL OR ip.dev_seconds = 0
           OR ip.lead_seconds IS NULL OR ip.lead_seconds <= 0,
           CAST(NULL AS Nullable(Float64)),
           CAST(toFloat64(ip.dev_seconds) AS Nullable(Float64)))),
    ('flow_efficiency_den',
        if(ip.dev_seconds IS NULL OR ip.dev_seconds = 0
           OR ip.lead_seconds IS NULL OR ip.lead_seconds <= 0,
           CAST(NULL AS Nullable(Float64)),
           CAST(toFloat64(ip.lead_seconds) AS Nullable(Float64)))),
    ('pickup_time',
        if(ip.pickup_seconds IS NULL,
           CAST(NULL AS Nullable(Float64)),
           CAST(round(toFloat64(ip.pickup_seconds) / 86400.0, 2)
                AS Nullable(Float64))))
] AS kv

UNION ALL

SELECT
    c.assignee_email                                             AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    c.event_date                                                 AS metric_date,
    'task_reopen_rate'                                           AS metric_key,
    CAST(toFloat64(c.close_count) AS Nullable(Float64))          AS metric_value
FROM insight.task_close_events_daily AS c
LEFT JOIN insight.people AS p ON c.assignee_email = p.person_id

UNION ALL

SELECT
    r.assignee_email                                             AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    r.event_date                                                 AS metric_date,
    'task_reopen_rate'                                           AS metric_key,
    CAST(-toFloat64(r.reopen_count) AS Nullable(Float64))        AS metric_value
FROM insight.task_reopen_events_daily AS r
LEFT JOIN insight.people AS p ON r.assignee_email = p.person_id

UNION ALL

SELECT
    coalesce(w.author_email, ip.assignee_email)                  AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    coalesce(w.work_date, ip.day)                                AS metric_date,
    kv.1                                                         AS metric_key,
    kv.2                                                         AS metric_value
FROM insight.task_worklog_seconds_per_day AS w
FULL OUTER JOIN insight.task_in_progress_seconds_per_day AS ip
    ON w.author_email = ip.assignee_email AND w.work_date = ip.day
LEFT JOIN insight.people AS p
    ON p.person_id = coalesce(w.author_email, ip.assignee_email)
ARRAY JOIN [
    ('worklog_seconds',
        if(ifNull(ip.in_progress_seconds, toFloat64(0)) > 0,
           CAST(toFloat64(ifNull(w.worklog_seconds, toFloat64(0)))
                AS Nullable(Float64)),
           CAST(NULL AS Nullable(Float64)))),
    ('in_progress_seconds',
        if(ifNull(ip.in_progress_seconds, toFloat64(0)) > 0,
           CAST(toFloat64(ip.in_progress_seconds) AS Nullable(Float64)),
           CAST(NULL AS Nullable(Float64))))
] AS kv

UNION ALL

-- stale_in_progress: currently-open (NOT done) issues idle >14 days.
-- Open = status_category != 'done' (or unmapped/NULL), replacing the
-- display-name NOT-IN list.
SELECT
    s.assignee_email                                             AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    today()                                                      AS metric_date,
    'stale_in_progress'                                          AS metric_key,
    CAST(toFloat64(count()) AS Nullable(Float64))                AS metric_value
FROM insight.task_issue_current_state AS s
LEFT JOIN insight.people AS p ON s.assignee_email = p.person_id
WHERE (s.status_category IS NULL OR s.status_category != 'done')
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
  AND s.last_status_event_at IS NOT NULL
  AND dateDiff('day', s.last_status_event_at, now()) > 14
GROUP BY s.assignee_email, p.org_unit_id;
