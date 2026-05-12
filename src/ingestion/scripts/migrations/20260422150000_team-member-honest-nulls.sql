-- Stop defaulting null values to "plausible" numbers in insight.team_member.
-- Consistent with 20260422100000 (ic_kpis-honest-nulls): when a source isn't
-- wired, expose NULL so the FE renders ComingSoon / skips alerts instead of
-- flagging the person as "Action Needed" based on a synthetic value.
--
-- Previous behavior buried missing signals:
--   dev_time_h      — NULL default → 8h  ("normal workday")
--   focus_time_pct  — NULL default → 100 ("perfect focus")
--   ai_loc_share_pct— NULL default → 0
--
-- This migration makes these columns honestly Nullable and emits NULL
-- when the upstream source is absent. The analytics-api TEAM_MEMBER seed
-- uses `avg()`/`sum()` which skip NULLs, so aggregation stays correct.
--
-- `ai_tools` stays Array(String) — ClickHouse disallows Nullable(Array).
-- The FE gates "not using AI" alerts on `availability.ai === 'available'`
-- from the Connector Manager instead of per-person nullability.
--
-- Depends on 20260422000000_gold-views.sql (people/jira_closed_tasks exist).

DROP VIEW IF EXISTS insight.team_member;

CREATE VIEW insight.team_member AS
SELECT
    p.person_id                                                             AS person_id,
    p.display_name                                                          AS display_name,
    p.seniority                                                             AS seniority,
    p.org_unit_id                                                           AS org_unit_id,
    toFloat64(ifNull(j.tasks_closed, 0))                                    AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0))                                      AS bugs_fixed,
    -- Keep `greatest(0, ...)` to clamp negative outliers, but without
    -- `ifNull(..., 8)` — NULL dev_time_h now stays NULL.
    if(f.dev_time_h IS NULL,
        CAST(NULL AS Nullable(Float64)),
        CAST(greatest(0, round(f.dev_time_h, 1)) AS Nullable(Float64)))     AS dev_time_h,
    toFloat64(0)                                                            AS prs_merged,
    CAST(NULL AS Nullable(Float64))                                         AS build_success_pct,
    -- 0..100 clamp retained; NULL focus_time_pct now stays NULL.
    if(f.focus_time_pct IS NULL,
        CAST(NULL AS Nullable(Float64)),
        CAST(greatest(0, least(100, round(f.focus_time_pct, 1)))
             AS Nullable(Float64)))                                          AS focus_time_pct,
    -- ai_tools stays non-Nullable: ClickHouse forbids Nullable(Array(T)).
    -- FE gates AI-related alerts via data_availability.ai instead.
    if(ifNull(cur.is_active, 0) = 1,
        ['Cursor'],
        CAST([] AS Array(String)))                                          AS ai_tools,
    -- ai_loc_share_pct: NULL when the cursor row is missing (as opposed
    -- to 0 which now means "connected, but no accepted lines this day").
    if(cur.person_id IS NULL,
        CAST(NULL AS Nullable(Float64)),
        CAST(round(cur.ai_loc_share_pct, 1) AS Nullable(Float64)))          AS ai_loc_share_pct,
    f.metric_date                                                           AS metric_date
FROM insight.people AS p
INNER JOIN
(
    SELECT
        email,
        toString(day)     AS metric_date,
        focus_time_pct,
        dev_time_h
    FROM silver.class_focus_metrics
) AS f ON p.person_id = f.email
LEFT JOIN
(
    SELECT
        person_id,
        toString(metric_date)   AS metric_date,
        sum(tasks_closed)       AS tasks_closed,
        sum(bugs_fixed)         AS bugs_fixed
    FROM insight.jira_closed_tasks
    GROUP BY person_id, metric_date
) AS j ON (p.person_id = j.person_id) AND (f.metric_date = j.metric_date)
LEFT JOIN
(
    SELECT
        lower(email)            AS person_id,
        day                     AS metric_date,
        if(isActive = true, 1, 0) AS is_active,
        if(toFloat64(ifNull(totalLinesAdded, 0)) > 0,
           round((toFloat64(ifNull(acceptedLinesAdded, 0)) /
                  toFloat64(totalLinesAdded)) * 100, 1),
           0)                   AS ai_loc_share_pct
    FROM bronze_cursor.cursor_daily_usage
) AS cur ON (p.person_id = cur.person_id) AND (f.metric_date = cur.metric_date)
WHERE p.status = 'Active';
