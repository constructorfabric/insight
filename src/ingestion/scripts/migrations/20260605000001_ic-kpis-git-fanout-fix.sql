-- ---------------------------------------------------------------------
-- ic_kpis — fix git-metric join fanout (prs_merged, loc)
-- ---------------------------------------------------------------------
-- BUG (introduced 20260427120000): git metrics (prs_merged, loc,
-- pr_cycle_time_h) were LEFT JOINed from silver.mtr_git_person_totals — ONE
-- all-time row per person — onto the per-(person,day) class_focus_metrics
-- driver with NO date key. The same lifetime value repeated on every focus
-- day, so the metric query's sum(prs_merged)/sum(loc) multiplied the lifetime
-- total by the person's focus-day count (e.g. prs_merged 11 -> 209 = 11×19;
-- loc inflated identically). pr_cycle_time_h (avg of a constant) didn't fan
-- out but ignored the selected period (always all-time).
--
-- FIX: git metrics now enter as their OWN per-(person, week-start) fact rows
-- (UNION ALL branch) sourced from silver.mtr_git_person_weekly (period-bounded,
-- date-keyed). The metric query's sum over a metric_date range therefore counts
-- each week exactly once — no fanout, and the value reflects the selected
-- period. The daily branch (focus / jira / cursor) is unchanged and carries
-- NULL for the git columns so they don't dilute sums/avgs. pr_cycle_time_h is
-- dropped here (NULL): the period-correct PR cycle time already lives in
-- git_bullet_rows (the IC git bullet section); it was never a sound KPI value
-- from the all-time totals table. ai_loc_share_pct is left as-is (a separate,
-- known measurement question — not changed in this migration).
DROP VIEW IF EXISTS insight.ic_kpis;
CREATE VIEW insight.ic_kpis AS
SELECT
    f.email                                       AS person_id,
    p.org_unit_id                                 AS org_unit_id,
    toString(f.day)                               AS metric_date,
    CAST(NULL AS Nullable(Float64))               AS loc,
    round(ifNull(cur.ai_loc_share_pct, 0), 1)     AS ai_loc_share_pct,
    CAST(NULL AS Nullable(Float64))               AS prs_merged,
    CAST(NULL AS Nullable(Float64))               AS pr_cycle_time_h,
    greatest(0, least(100, round(ifNull(f.focus_time_pct, 100), 1)))
                                                  AS focus_time_pct,
    toFloat64(ifNull(j.tasks_closed, 0))          AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0))            AS bugs_fixed,
    CAST(NULL, 'Nullable(Float64)')               AS build_success_pct,
    toFloat64(ifNull(cur.ai_sessions, 0))         AS ai_sessions
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id
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
) AS cur ON (f.email = cur.person_id) AND (toString(f.day) = cur.metric_date)

UNION ALL

-- Git facts: per (person, week-start). Period-bounded; one row per week so the
-- metric query's sum() over a date range counts each week once (no fanout).
SELECT
    g.person_key                                  AS person_id,
    p.org_unit_id                                 AS org_unit_id,
    toString(g.week)                              AS metric_date,
    CAST(toFloat64(g.code_loc) AS Nullable(Float64))   AS loc,
    CAST(NULL AS Nullable(Float64))               AS ai_loc_share_pct,
    CAST(toFloat64(g.prs_merged) AS Nullable(Float64)) AS prs_merged,
    CAST(NULL AS Nullable(Float64))               AS pr_cycle_time_h,
    CAST(NULL AS Nullable(Float64))               AS focus_time_pct,
    CAST(NULL AS Nullable(Float64))               AS tasks_closed,
    CAST(NULL AS Nullable(Float64))               AS bugs_fixed,
    CAST(NULL AS Nullable(Float64))               AS build_success_pct,
    CAST(NULL AS Nullable(Float64))               AS ai_sessions
FROM silver.mtr_git_person_weekly AS g
INNER JOIN insight.people AS p ON g.person_key = p.person_id
WHERE p.status = 'Active' AND g.week IS NOT NULL;
