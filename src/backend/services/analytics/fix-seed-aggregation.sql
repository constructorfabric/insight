-- Fix: TEAM_MEMBER and IC_KPIS seed queries lack GROUP BY.
--
-- Problem: both query_ref values returned per-(person, date) rows from the
-- ClickHouse views.  The FE expects one row per person with period-level
-- aggregates.  Without GROUP BY the team table showed duplicate members and
-- the IC dashboard displayed a single arbitrary day instead of the full
-- period.
--
-- This script updates the query_ref column in the metrics table directly.
-- Run against the analytics MariaDB database.

-- TEAM_MEMBER  (00000000-0000-0000-0001-000000000002)
UPDATE metrics
SET query_ref = 'SELECT person_id, any(display_name) AS display_name, any(seniority) AS seniority, org_unit_id, sum(tasks_closed) AS tasks_closed, sum(bugs_fixed) AS bugs_fixed, round(avg(dev_time_h), 1) AS dev_time_h, sum(prs_merged) AS prs_merged, avg(build_success_pct) AS build_success_pct, round(avg(focus_time_pct), 1) AS focus_time_pct, any(ai_tools) AS ai_tools, round(avg(ai_loc_share_pct), 1) AS ai_loc_share_pct FROM insight.team_member GROUP BY person_id, org_unit_id'
WHERE id = UNHEX('00000000000000000001000000000002');

-- IC_KPIS  (00000000-0000-0000-0001-000000000010)
UPDATE metrics
SET query_ref = 'SELECT person_id, sum(loc) AS loc, round(avg(ai_loc_share_pct), 1) AS ai_loc_share_pct, sum(prs_merged) AS prs_merged, round(avg(pr_cycle_time_h), 1) AS pr_cycle_time_h, round(avg(focus_time_pct), 1) AS focus_time_pct, sum(tasks_closed) AS tasks_closed, sum(bugs_fixed) AS bugs_fixed, avg(build_success_pct) AS build_success_pct, sum(ai_sessions) AS ai_sessions FROM insight.ic_kpis GROUP BY person_id'
WHERE id = UNHEX('00000000000000000001000000000010');
