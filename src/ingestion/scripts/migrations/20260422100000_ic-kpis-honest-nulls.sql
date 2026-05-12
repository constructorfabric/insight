-- Emit NULL (not hardcoded 0) for KPI columns whose source data isn't
-- ingested yet. Lets the FE render a ComingSoon placeholder for "unknown"
-- instead of displaying a fake zero that looks like a real measurement.
--
-- Depends on 20260422000000_gold-views.sql (insight.ic_kpis and
-- insight.ic_chart_delivery already exist).
--
-- Columns made Nullable and set to NULL:
--   insight.ic_kpis.loc               — real git-LOC needs Bitbucket diffstat
--                                       (not in bronze_bitbucket_cloud.commits yet).
--                                       Previously sourced from Cursor's
--                                       totalLinesAdded, which is IDE-typed
--                                       activity, not committed code.
--   insight.ic_kpis.prs_merged        — Bitbucket PR ingestion not landed.
--   insight.ic_kpis.pr_cycle_time_h   — same.
--   insight.ic_chart_delivery.prs_merged — same.
--
-- When a real source lands, just flip the column expression back to its
-- real aggregate — the column type (Nullable) already accommodates both.
--
-- Fresh-cluster bring-up: this migration plus #4–#7 SELECT from silver
-- dbt-models and bronze connector tables that don't exist on first
-- install. Minimum-viable placeholders for every such table are
-- created earlier in the init flow by
-- `scripts/create-bronze-placeholders.sh` (extended to cover silver
-- per the bring-up convention — see ADR-0007). dbt's first run
-- replaces them with the real models / Airbyte syncs do the same for
-- bronze. ClickHouse VIEW resolution rebinds on each SELECT.

-- =====================================================================
-- insight.ic_kpis — nullable loc/prs_merged/pr_cycle_time_h
-- =====================================================================
DROP VIEW IF EXISTS insight.ic_kpis;

CREATE VIEW insight.ic_kpis AS
SELECT
    f.email                                                             AS person_id,
    p.org_unit_id                                                       AS org_unit_id,
    toString(f.day)                                                     AS metric_date,
    CAST(NULL, 'Nullable(Float64)')                                     AS loc,
    round(ifNull(cur.ai_loc_share_pct, 0), 1)                           AS ai_loc_share_pct,
    CAST(NULL, 'Nullable(Float64)')                                     AS prs_merged,
    CAST(NULL, 'Nullable(Float64)')                                     AS pr_cycle_time_h,
    greatest(0, least(100, round(ifNull(f.focus_time_pct, 100), 1)))    AS focus_time_pct,
    toFloat64(ifNull(j.tasks_closed, 0))                                AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0))                                  AS bugs_fixed,
    CAST(NULL, 'Nullable(Float64)')                                     AS build_success_pct,
    toFloat64(ifNull(cur.ai_sessions, 0))                               AS ai_sessions
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id
LEFT JOIN
(
    SELECT
        person_id,
        toString(metric_date)                                           AS metric_date,
        sum(tasks_closed)                                               AS tasks_closed,
        sum(bugs_fixed)                                                 AS bugs_fixed
    FROM insight.jira_closed_tasks
    GROUP BY person_id, metric_date
) AS j ON (f.email = j.person_id) AND (toString(f.day) = j.metric_date)
LEFT JOIN
(
    SELECT
        lower(email)                                                    AS person_id,
        day                                                             AS metric_date,
        if(toFloat64(ifNull(totalLinesAdded, 0)) > 0,
           round((toFloat64(ifNull(acceptedLinesAdded, 0)) /
                  toFloat64(totalLinesAdded)) * 100, 1),
           0)                                                           AS ai_loc_share_pct,
        (toFloat64(ifNull(agentRequests, 0)) +
         toFloat64(ifNull(chatRequests, 0))) +
         toFloat64(ifNull(composerRequests, 0))                         AS ai_sessions
    FROM bronze_cursor.cursor_daily_usage
) AS cur ON (f.email = cur.person_id) AND (toString(f.day) = cur.metric_date);

-- =====================================================================
-- insight.ic_chart_delivery — nullable prs_merged
-- =====================================================================
DROP VIEW IF EXISTS insight.ic_chart_delivery;

CREATE VIEW insight.ic_chart_delivery AS
WITH
    weekly_commits AS (
        SELECT
            person_id,
            toStartOfWeek(metric_date)                                  AS week,
            sum(commits)                                                AS commits
        FROM insight.commits_daily
        GROUP BY person_id, week
    ),
    weekly_jira AS (
        SELECT
            person_id,
            toStartOfWeek(metric_date)                                  AS week,
            sum(tasks_closed)                                           AS tasks_done
        FROM insight.jira_closed_tasks
        GROUP BY person_id, week
    ),
    weeks_all AS (
        SELECT person_id, week FROM weekly_commits
        UNION DISTINCT
        SELECT person_id, week FROM weekly_jira
    )
SELECT
    d.person_id                                                         AS person_id,
    p.org_unit_id                                                       AS org_unit_id,
    toString(d.week)                                                    AS date_bucket,
    toString(d.week)                                                    AS metric_date,
    toUInt64(ifNull(c.commits, 0))                                      AS commits,
    CAST(NULL, 'Nullable(UInt64)')                                      AS prs_merged,
    toUInt64(ifNull(j.tasks_done, 0))                                   AS tasks_done
FROM weeks_all d
LEFT JOIN insight.people  p ON d.person_id = p.person_id
LEFT JOIN weekly_commits  c ON c.person_id = d.person_id AND c.week = d.week
LEFT JOIN weekly_jira     j ON j.person_id = d.person_id AND j.week = d.week;
