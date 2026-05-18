-- =====================================================================
-- task_delivery_bullet_rows — Phase A rewrite (issue #433 §4.1)
-- =====================================================================
--
-- This migration replaces the UNION-ALL-heavy view with a slimmer, faster
-- shape that also unlocks mathematically correct ratio aggregation
-- downstream. Changes vs the predecessor migration
-- (`20260429000000_task-delivery-silver-rewrite.sql`):
--
--   1. SCAN CONSOLIDATION (issue #433 §3.5). The view dropped from 12
--      UNION-ALL branches to 5 — one per source table. Within each branch,
--      multiple metrics are now emitted via `ARRAY JOIN` over a tuple
--      array, so the source table is read ONCE and the row is then
--      unpacked into N rows (one per metric). The previous shape made
--      ClickHouse re-scan `jira_closed_tasks` four times and
--      `task_dev_seconds_per_issue` four times — visible at MVP scale,
--      O(N) cost per render at production scale.
--
--   2. RATIO num/den SPLIT (issue #433 §3.3). The four daily-ratio
--      metrics that were computed inline in the previous view
--      (`due_date_compliance`, `bugs_to_task_ratio`, `flow_efficiency`,
--      `worklog_logging_accuracy`) are gone from the view as
--      first-class `metric_key`s. The view now emits raw numerator and
--      denominator counters instead:
--
--        due_date_compliance      → due_date_on_time + due_date_with_due
--        bugs_to_task_ratio       → bugs_fixed (denom reuses tasks_completed)
--        flow_efficiency          → flow_efficiency_num + flow_efficiency_den
--        worklog_logging_accuracy → worklog_seconds + in_progress_seconds
--
--      The composite ratios are reconstructed in `query_ref` as
--      `100 * Σnum / Σden`, which is the only mathematically correct
--      definition when daily denominators differ (CLAUDE.md
--      "Aggregation correctness"). The previous shape stored
--      `daily_pct` in the view and downstream took `avg(daily_pct)`,
--      systematically biasing toward days with small task counts.
--
--      `estimation_accuracy` is NOT split into num/den here. The silver
--      layer only exposes `avg_time_estimate` / `avg_time_spent` per
--      person-day (already-averaged scalars), not the per-task arrays
--      needed for a clean Σnum/Σden. The view continues to emit the
--      daily percentage; the symmetric-folding aggregation
--      (`100 - avg(|100 - pct|)`) in `query_ref` is preserved as-is.
--      Promoting it to true Σnum/Σden requires changing the silver
--      `jira_closed_tasks` shape — separate workstream.
--
--   3. `metric_date` type. Previously `String` via `toString(...)`;
--      now `Date` (the native type of each source column). This
--      unlocks MergeTree min/max statistics on downstream
--      `class_collab_*`-style consumers, makes future partition keys
--      possible, and removes a layer of accidental lexical-vs-date
--      coupling for any consumer doing range comparisons. No external
--      API exposes this column; downstream `query_ref` reads
--      `WHERE metric_date >= '2026-04-01'` which works identically on
--      Date and String columns when the literal is an ISO-8601 date.
--
-- Branch shape after rewrite (5 branches, source-aligned):
--
--   1. `jira_closed_tasks`            → 5 keys via ARRAY JOIN
--   2. `task_dev_seconds_per_issue`   → 5 keys via ARRAY JOIN
--   3. `task_close_events_daily`      → 1 key (`+task_reopen_rate`)
--   4. `task_reopen_events_daily`     → 1 key (`-task_reopen_rate`)
--      [3+4 are the existing signed-event pattern, NOT consolidated
--       because they read from two different source tables.]
--   5. `task_worklog_seconds_per_day` ⋈ `task_in_progress_seconds_per_day`
--                                     → 2 keys via ARRAY JOIN (worklog num/den)
--   6. `task_issue_current_state`     → 1 key (`stale_in_progress` snapshot)
--
-- 15 distinct metric_keys after rewrite. Composite-ratio metric_keys
-- visible on FE (`due_date_compliance`, `bugs_to_task_ratio`,
-- `flow_efficiency`, `worklog_logging_accuracy`) live ONLY in the
-- `query_ref` projection — they are not emitted by the view.
-- =====================================================================

DROP VIEW IF EXISTS insight.task_delivery_bullet_rows;

CREATE VIEW insight.task_delivery_bullet_rows AS

-- ─── Branch 1: jira_closed_tasks (per-person-per-day aggregate) ──────
-- Emits 5 keys: tasks_completed, due_date_on_time, due_date_with_due,
-- estimation_accuracy (daily %; see header §2 caveat), bugs_fixed.
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

    -- estimation_accuracy: daily % (estimate/spent)*100, NULL when
    -- spent <= 0 or estimate missing. The silver layer stores avg_*
    -- scalars per person-day, not per-task arrays, so a true Σnum/Σden
    -- isn't possible without changing silver. query_ref applies
    -- symmetric folding (100 - avg(|100 - daily%|)) over valid days
    -- using `avgIf`/`countIf` over the non-NULL rows directly — no
    -- separate "samples" sibling metric is needed.
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

-- ─── Branch 2: task_dev_seconds_per_issue (per-issue grain) ──────────
-- Emits 5 keys: task_dev_time, mean_time_to_resolution,
-- flow_efficiency_num, flow_efficiency_den, pickup_time.
-- One row per closed issue in source → 5 long rows after ARRAY JOIN.
-- query_ref takes period-level avg over issues for time-based metrics,
-- Σnum/Σden for flow_efficiency.
SELECT
    ip.assignee_email                                            AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    ip.close_date                                                AS metric_date,
    kv.1                                                         AS metric_key,
    kv.2                                                         AS metric_value
FROM insight.task_dev_seconds_per_issue AS ip
LEFT JOIN insight.people AS p ON ip.assignee_email = p.person_id
ARRAY JOIN [
    -- task_dev_time: hours in dev statuses for this issue. NULL when
    -- the issue had no recorded dev time. query_ref takes period-level
    -- avg across issues so a single year-old issue closed in window
    -- doesn't drag the team value up.
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

    -- flow_efficiency num/den: per-issue dev_seconds and lead_seconds.
    -- Both emitted only when the pair is internally consistent
    -- (dev > 0 AND lead > 0). Otherwise both NULL so the period
    -- aggregate stays balanced and doesn't divide a partially-NULL
    -- numerator by a fully-populated denominator (or vice versa).
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

-- ─── Branch 3a: task_reopen_rate close events (positive sign) ────────
-- The signed-event pattern is preserved from the predecessor. query_ref
-- reconstructs the rate as `-Σ(neg) / Σ(pos) × 100`.
SELECT
    c.assignee_email                                             AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    c.event_date                                                 AS metric_date,
    'task_reopen_rate'                                           AS metric_key,
    CAST(toFloat64(c.close_count) AS Nullable(Float64))          AS metric_value
FROM insight.task_close_events_daily AS c
LEFT JOIN insight.people AS p ON c.assignee_email = p.person_id

UNION ALL

-- ─── Branch 3b: task_reopen_rate reopen events (negative sign) ───────
SELECT
    r.assignee_email                                             AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    r.event_date                                                 AS metric_date,
    'task_reopen_rate'                                           AS metric_key,
    CAST(-toFloat64(r.reopen_count) AS Nullable(Float64))        AS metric_value
FROM insight.task_reopen_events_daily AS r
LEFT JOIN insight.people AS p ON r.assignee_email = p.person_id

UNION ALL

-- ─── Branch 4: worklog ⋈ in_progress (FULL OUTER JOIN) ───────────────
-- Emits 2 keys: worklog_seconds, in_progress_seconds.
-- The pair is only emitted when in_progress_seconds > 0 — the
-- denominator-guard matches the predecessor's behavior and prevents
-- emitting (worklog=N, in_progress=0) pairs that would zero-divide.
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

-- ─── Branch 5: stale_in_progress (snapshot, today's date) ────────────
-- Count of currently-open issues whose last status event is >14 days
-- old. Emitted only once with metric_date = today() since the metric
-- is a point-in-time snapshot, not period-aggregable. query_ref
-- sums these per person.
SELECT
    s.assignee_email                                             AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    today()                                                      AS metric_date,
    'stale_in_progress'                                          AS metric_key,
    CAST(toFloat64(count()) AS Nullable(Float64))                AS metric_value
FROM insight.task_issue_current_state AS s
LEFT JOIN insight.people AS p ON s.assignee_email = p.person_id
WHERE (s.status_name IS NULL OR s.status_name NOT IN ('Closed','Resolved','Verified'))
  AND s.assignee_email IS NOT NULL
  AND s.assignee_email != ''
  AND s.last_status_event_at IS NOT NULL
  AND dateDiff('day', s.last_status_event_at, now()) > 14
GROUP BY s.assignee_email, p.org_unit_id;
