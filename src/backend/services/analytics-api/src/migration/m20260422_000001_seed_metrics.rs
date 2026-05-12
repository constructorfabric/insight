//! Seed metric definitions for all FE dashboard views (squashed).
//!
//! Source: maria-metrics-alexey.sql (2026-04-22)
//! Replaces: `m20260417_000001_seed_metrics` + `m20260421_000001_seed_git_metric`
//!
//! UUIDs match `insight-front/src/screensets/insight/api/metricRegistry.ts`.
//! Each metric's `query_ref` points to a `ClickHouse` view in the `insight` DB
//! created by `20260422000000_gold-views.sql`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Metric seed row: (`hex_id`, name, description, `query_ref`).
const SEEDS: &[(&str, &str, &str, &str)] = &[
    // ─── EXEC_SUMMARY ──────────────────────────────────────────────
    (
        "00000000000000000001000000000001",
        "Executive Summary",
        "Org-unit level summary: headcount, tasks, bugs, focus, AI adoption, PR cycle time",
        "SELECT org_unit_id, any(org_unit_name) AS org_unit_name, any(headcount) AS headcount, sum(tasks_closed) AS tasks_closed, sum(bugs_fixed) AS bugs_fixed, anyOrNull(build_success_pct) AS build_success_pct, round(avg(focus_time_pct), 1) AS focus_time_pct, round(avg(ai_adoption_pct), 1) AS ai_adoption_pct, round(avg(ai_loc_share_pct), 1) AS ai_loc_share_pct, avg(pr_cycle_time_h) AS pr_cycle_time_h FROM insight.exec_summary GROUP BY org_unit_id",
    ),
    // ─── TEAM_MEMBER ───────────────────────────────────────────────
    // Joins insight.people for job_title override (VP/Chief → Senior) and a
    // bamboohr subquery for supervisor_email (needed for the FE's
    // direct-reports-only toggle). Preserves the actual ai_tools array
    // instead of hardcoding ['Cursor'] — multi-source AI support.
    (
        "00000000000000000001000000000002",
        "Team Members",
        "Per-person metrics for team view: tasks, bugs, dev time, PRs, focus, AI tools, supervisor_email",
        "SELECT m.person_id AS person_id, any(m.display_name) AS display_name, multiIf(any(p.job_title) ILIKE '%vice president%' OR any(p.job_title) ILIKE '%chief%' OR any(p.job_title) ILIKE '%president%' OR any(p.job_title) ILIKE '%ceo%' OR any(p.job_title) ILIKE '%cto%' OR any(p.job_title) ILIKE '%cfo%' OR any(p.job_title) ILIKE '%coo%' OR any(p.job_title) ILIKE '%vp of%' OR any(p.job_title) ILIKE 'vp %' OR any(p.job_title) ILIKE '%executive%', 'Senior', any(m.seniority)) AS seniority, m.org_unit_id AS org_unit_id, any(s.supervisor_email) AS supervisor_email, sum(m.tasks_closed) AS tasks_closed, sum(m.bugs_fixed) AS bugs_fixed, round(avg(m.dev_time_h), 1) AS dev_time_h, sum(m.prs_merged) AS prs_merged, anyOrNull(m.build_success_pct) AS build_success_pct, round(avg(m.focus_time_pct), 1) AS focus_time_pct, arrayDistinct(arrayFlatten(groupArray(m.ai_tools))) AS ai_tools, round(avg(m.ai_loc_share_pct), 1) AS ai_loc_share_pct FROM insight.team_member m LEFT JOIN insight.people p ON m.person_id = p.person_id LEFT JOIN (SELECT lower(workEmail) AS person_id, lower(argMax(supervisorEmail, _airbyte_extracted_at)) AS supervisor_email FROM bronze_bamboohr.employees WHERE workEmail IS NOT NULL AND workEmail != '' GROUP BY lower(workEmail)) s ON s.person_id = m.person_id GROUP BY m.person_id, m.org_unit_id",
    ),
    // ─── TEAM BULLETS — value=team (filtered by org_unit_id), range=company ──
    // Outer aggregate gives the team's value; `c` subquery aggregates across
    // ALL people for the company-wide median/min/max used as the bullet range.
    // FE reads `median`/`range_min`/`range_max` (see transforms.ts).
    (
        "00000000000000000001000000000003",
        "Team Bullet Task Delivery",
        "Bullet chart metrics for task delivery. value=team avg, range=company min/max",
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.company_median) AS median, any(c.company_min) AS range_min, any(c.company_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key = 'tasks_completed', sum(metric_value), metric_key = 'estimation_accuracy', if(countIf(metric_value > 0 AND metric_value <= 200) > 0, greatest(toFloat64(0), toFloat64(100) - avgIf(abs(toFloat64(100) - metric_value), metric_value > 0 AND metric_value <= 200)), NULL), avg(metric_value)) AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, quantileExact(0.5)(v_period) AS company_median, min(v_period) AS company_min, max(v_period) AS company_max FROM (SELECT metric_key, person_id, multiIf(metric_key = 'tasks_completed', sum(metric_value), metric_key = 'estimation_accuracy', if(countIf(metric_value > 0 AND metric_value <= 200) > 0, greatest(toFloat64(0), toFloat64(100) - avgIf(abs(toFloat64(100) - metric_value), metric_value > 0 AND metric_value <= 200)), NULL), avg(metric_value)) AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key) c ON c.metric_key = p.metric_key GROUP BY p.metric_key",
    ),
    (
        "00000000000000000001000000000004",
        "Team Bullet Code Quality",
        "Bullet chart metrics for code quality. value=team avg, range=company min/max",
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.company_median) AS median, any(c.company_min) AS range_min, any(c.company_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('bugs_fixed', 'prs_per_dev'), sum(metric_value), avg(metric_value)) AS v_period FROM insight.code_quality_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, quantileExact(0.5)(v_period) AS company_median, min(v_period) AS company_min, max(v_period) AS company_max FROM (SELECT metric_key, person_id, multiIf(metric_key IN ('bugs_fixed', 'prs_per_dev'), sum(metric_value), avg(metric_value)) AS v_period FROM insight.code_quality_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key) c ON c.metric_key = p.metric_key GROUP BY p.metric_key",
    ),
    (
        "00000000000000000001000000000005",
        "Team Bullet Collaboration",
        "Bullet chart metrics for collaboration. value=team avg, range=company min/max",
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.company_median) AS median, any(c.company_min) AS range_min, any(c.company_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('m365_emails_sent', 'zoom_calls', 'meeting_hours', 'm365_teams_messages', 'm365_files_shared', 'meeting_free', 'slack_thread_participation', 'slack_message_engagement'), sum(metric_value), avg(metric_value)) AS v_period FROM insight.collab_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, quantileExact(0.5)(v_period) AS company_median, min(v_period) AS company_min, max(v_period) AS company_max FROM (SELECT metric_key, person_id, multiIf(metric_key IN ('m365_emails_sent', 'zoom_calls', 'meeting_hours', 'm365_teams_messages', 'm365_files_shared', 'meeting_free', 'slack_thread_participation', 'slack_message_engagement'), sum(metric_value), avg(metric_value)) AS v_period FROM insight.collab_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key) c ON c.metric_key = p.metric_key GROUP BY p.metric_key",
    ),
    (
        "00000000000000000001000000000006",
        "Team Bullet AI Adoption",
        "Bullet chart metrics for AI adoption. value=team, range=company. Active indicators use count() as scale",
        "SELECT p.metric_key AS metric_key, multiIf(p.metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), sum(p.v_period), avg(p.v_period)) AS value, any(c.company_median) AS median, any(c.company_min) AS range_min, any(c.company_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), avg(metric_value)) AS v_period FROM insight.ai_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(0), quantileExact(0.5)(v_period)) AS company_median, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(0), min(v_period)) AS company_min, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(count()), max(v_period)) AS company_max FROM (SELECT metric_key, person_id, multiIf(metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), avg(metric_value)) AS v_period FROM insight.ai_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key) c ON c.metric_key = p.metric_key GROUP BY p.metric_key",
    ),
    // ─── IC KPIS ───────────────────────────────────────────────────
    (
        "00000000000000000001000000000010",
        "IC KPIs",
        "Per-person KPI aggregates",
        "SELECT person_id, sum(loc) AS loc, round(avg(ai_loc_share_pct), 1) AS ai_loc_share_pct, sum(prs_merged) AS prs_merged, avg(pr_cycle_time_h) AS pr_cycle_time_h, round(avg(focus_time_pct), 1) AS focus_time_pct, sum(tasks_closed) AS tasks_closed, sum(bugs_fixed) AS bugs_fixed, anyOrNull(build_success_pct) AS build_success_pct, sum(ai_sessions) AS ai_sessions FROM insight.ic_kpis GROUP BY person_id",
    ),
    // ─── IC BULLETS — value=person (filtered by person_id), range=their team ──
    // `c` subquery groups by (metric_key, org_unit_id) and the outer JOIN
    // matches on both — so each person's bullet range reflects their own
    // team's distribution, not the whole company. Consistent with the
    // IC_BULLET_GIT pattern in UUID …18.
    (
        "00000000000000000001000000000011",
        "IC Bullet Task Delivery",
        "IC-level bullet metrics for task delivery. value=person, range=team (org_unit) min/max",
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key = 'tasks_completed', sum(metric_value), metric_key = 'estimation_accuracy', if(countIf(metric_value > 0 AND metric_value <= 200) > 0, greatest(toFloat64(0), toFloat64(100) - avgIf(abs(toFloat64(100) - metric_value), metric_value > 0 AND metric_value <= 200)), NULL), avg(metric_value)) AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExact(0.5)(v_period) AS team_median, min(v_period) AS team_min, max(v_period) AS team_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key = 'tasks_completed', sum(metric_value), metric_key = 'estimation_accuracy', if(countIf(metric_value > 0 AND metric_value <= 200) > 0, greatest(toFloat64(0), toFloat64(100) - avgIf(abs(toFloat64(100) - metric_value), metric_value > 0 AND metric_value <= 200)), NULL), avg(metric_value)) AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key",
    ),
    (
        "00000000000000000001000000000012",
        "IC Bullet Collaboration",
        "IC-level bullet metrics for collaboration. value=person, range=team min/max",
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('m365_emails_sent', 'zoom_calls', 'meeting_hours', 'm365_teams_messages', 'm365_files_shared', 'meeting_free', 'slack_thread_participation', 'slack_message_engagement'), sum(metric_value), avg(metric_value)) AS v_period FROM insight.collab_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExact(0.5)(v_period) AS team_median, min(v_period) AS team_min, max(v_period) AS team_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('m365_emails_sent', 'zoom_calls', 'meeting_hours', 'm365_teams_messages', 'm365_files_shared', 'meeting_free', 'slack_thread_participation', 'slack_message_engagement'), sum(metric_value), avg(metric_value)) AS v_period FROM insight.collab_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key",
    ),
    (
        "00000000000000000001000000000013",
        "IC Bullet AI",
        "IC-level bullet metrics for AI adoption. value=person, range=team. Active indicators use count() as scale",
        "SELECT p.metric_key AS metric_key, multiIf(p.metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), sum(p.v_period), avg(p.v_period)) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), avg(metric_value)) AS v_period FROM insight.ai_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, org_unit_id, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(0), quantileExact(0.5)(v_period)) AS team_median, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(0), min(v_period)) AS team_min, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(count()), max(v_period)) AS team_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), avg(metric_value)) AS v_period FROM insight.ai_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key",
    ),
    // ─── IC CHARTS ─────────────────────────────────────────────────
    (
        "00000000000000000001000000000014",
        "IC Chart LOC Trend",
        "Weekly LOC trend: AI-generated, manual code, spec lines",
        "SELECT date_bucket, ai_loc, code_loc, spec_lines, person_id, metric_date FROM insight.ic_chart_loc",
    ),
    (
        "00000000000000000001000000000015",
        "IC Chart Delivery Trend",
        "Weekly delivery trend: commits, PRs merged, tasks done",
        "SELECT date_bucket, commits, prs_merged, tasks_done, person_id, metric_date FROM insight.ic_chart_delivery",
    ),
    // ─── IC DRILL / TIMEOFF (placeholders) ─────────────────────────
    (
        "00000000000000000001000000000016",
        "IC Drill Detail",
        "Drill-down detail for IC metrics (placeholder)",
        "SELECT person_id, drill_id, title, source, src_class, value, filter, columns, rows, metric_date FROM insight.ic_drill",
    ),
    (
        "00000000000000000001000000000017",
        "IC Time Off",
        "Upcoming time off from BambooHR leave requests",
        "SELECT person_id, days, date_range, bamboo_hr_url, metric_date FROM insight.ic_timeoff",
    ),
    // ─── IC BULLET GIT ────────────────────────────────────────────
    (
        "00000000000000000001000000000018",
        "IC Bullet Git",
        "IC-level bullet metrics for git output (commits, ...). value=person, range=team min/max",
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, sum(metric_value) AS v_period FROM insight.git_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExact(0.5)(v_period) AS team_median, min(v_period) AS team_min, max(v_period) AS team_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, sum(metric_value) AS v_period FROM insight.git_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key",
    ),
];

const ZERO_TENANT: &str = "00000000000000000000000000000000";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        for (hex_id, name, description, query_ref) in SEEDS {
            db.execute_unprepared(&format!(
                "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
                 VALUES (UNHEX('{hex_id}'), UNHEX('{ZERO_TENANT}'), '{name}', '{description}', '{qr}', 1) \
                 ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref)",
                qr = query_ref.replace('\'', "''"),
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        for (hex_id, _, _, _) in SEEDS {
            db.execute_unprepared(&format!("DELETE FROM metrics WHERE id = UNHEX('{hex_id}')"))
                .await?;
        }

        Ok(())
    }
}
