//! Per-person "member metric values" `query_ref`s for the V2 team heatmap +
//! members-needing-attention widgets. One metric per section, all scoped by
//! the handler's `person_id IN (roster)`:
//!   - `…0040` Task Delivery, `…0041` Collaboration, `…0042` Git,
//!     `…0049` AI — each the section's `wide_aggregate_pp` rollup + `ARRAY JOIN`
//!     unpivot, emitting `(person_id, metric_key, value)` long rows (no cohort;
//!     the team view colors client-side against the department distribution).
//!   - `…0043` Member PRs Merged — per-person PRs merged from the canonical
//!     weekly silver (`silver.mtr_git_person_weekly`), the heatmap's PRs column
//!     (`git_bullet_rows.prs_merged` is empty and `team_member.prs_merged` is a
//!     NULL placeholder).
//!
//! The IC Bullet metrics aggregate per `metric_key` (collapsing people) and
//! join a department cohort — correct for the IC dashboard, but the team view
//! needs per-person values for the whole roster. `value` aggregation per
//! `metric_key` is copied verbatim from each section's bullet `query_ref`, so a
//! member's value here equals the value side of their IC bullet. The AI set
//! covers the distributable AI keys only (active-counter flags and NULL
//! placeholders excluded). `down()` deletes the metrics (append-only seed).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ZERO_TENANT: &str = "00000000000000000000000000000000";

const DELIVERY_HEX: &str = "00000000000000000001000000000040";
const COLLAB_HEX: &str = "00000000000000000001000000000041";
const GIT_HEX: &str = "00000000000000000001000000000042";
const MEMBER_PRS_HEX: &str = "00000000000000000001000000000043";
const MEMBER_VALUES_AI_HEX: &str = "00000000000000000001000000000049";

const DELIVERY_QR: &str = "SELECT person_id, kv.1 AS metric_key, kv.2 AS value FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, sumIf(metric_value, metric_key = 'tasks_completed') AS tasks_completed_v, sumIf(metric_value, metric_key = 'stale_in_progress') AS stale_in_progress_v, quantileExactIf(0.5)(metric_value, metric_key = 'task_dev_time' AND isNotNull(metric_value)) AS task_dev_time_v, quantileExactIf(0.5)(metric_value, metric_key = 'mean_time_to_resolution' AND isNotNull(metric_value)) AS mttr_v, quantileExactIf(0.5)(metric_value, metric_key = 'pickup_time' AND isNotNull(metric_value)) AS pickup_time_v, if(sumIf(metric_value, metric_key = 'task_reopen_rate' AND metric_value > 0) >= 5, round((-sumIf(metric_value, metric_key = 'task_reopen_rate' AND metric_value < 0) / sumIf(metric_value, metric_key = 'task_reopen_rate' AND metric_value > 0)) * 100, 1), CAST(NULL AS Nullable(Float64))) AS task_reopen_rate_v, if(sumIf(metric_value, metric_key = 'due_date_with_due') > 0, round(toFloat64(100) * sumIf(metric_value, metric_key = 'due_date_on_time') / sumIf(metric_value, metric_key = 'due_date_with_due'), 1), CAST(NULL AS Nullable(Float64))) AS due_date_compliance_v, if(sumIf(metric_value, metric_key = 'tasks_completed') > 0, round(toFloat64(100) * sumIf(metric_value, metric_key = 'bugs_fixed') / sumIf(metric_value, metric_key = 'tasks_completed'), 1), CAST(NULL AS Nullable(Float64))) AS bugs_to_task_ratio_v, if(sumIf(metric_value, metric_key = 'flow_efficiency_den') > 0, least(toFloat64(100), round(toFloat64(100) * sumIf(metric_value, metric_key = 'flow_efficiency_num') / sumIf(metric_value, metric_key = 'flow_efficiency_den'), 1)), CAST(NULL AS Nullable(Float64))) AS flow_efficiency_v, if(sumIf(metric_value, metric_key = 'in_progress_seconds') > 0, least(toFloat64(100), round(toFloat64(100) * sumIf(metric_value, metric_key = 'worklog_seconds') / sumIf(metric_value, metric_key = 'in_progress_seconds'), 1)), CAST(NULL AS Nullable(Float64))) AS worklog_logging_accuracy_v, if(countIf(metric_key = 'estimation_accuracy' AND metric_value > 0 AND metric_value <= 200) > 0, greatest(toFloat64(0), toFloat64(100) - avgIf(abs(toFloat64(100) - metric_value), metric_key = 'estimation_accuracy' AND metric_value > 0 AND metric_value <= 200)), CAST(NULL AS Nullable(Float64))) AS estimation_accuracy_v FROM insight.task_delivery_bullet_rows GROUP BY person_id) pp ARRAY JOIN [('tasks_completed', tasks_completed_v), ('stale_in_progress', stale_in_progress_v), ('task_dev_time', task_dev_time_v), ('mean_time_to_resolution', mttr_v), ('pickup_time', pickup_time_v), ('task_reopen_rate', task_reopen_rate_v), ('due_date_compliance', due_date_compliance_v), ('bugs_to_task_ratio', bugs_to_task_ratio_v), ('flow_efficiency', flow_efficiency_v), ('worklog_logging_accuracy', worklog_logging_accuracy_v), ('estimation_accuracy', estimation_accuracy_v)] AS kv";

const COLLAB_QR: &str = "SELECT person_id, kv.1 AS metric_key, kv.2 AS value FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, sumIf(metric_value, metric_key = 'm365_emails_sent') AS m365_emails_sent_v, sumIf(metric_value, metric_key = 'm365_emails_received') AS m365_emails_received_v, sumIf(metric_value, metric_key = 'm365_emails_read') AS m365_emails_read_v, sumIf(metric_value, metric_key = 'meeting_hours') AS meeting_hours_v, sumIf(metric_value, metric_key = 'meetings_count') AS meetings_count_v, sumIf(metric_value, metric_key = 'teams_meeting_hours') AS teams_meeting_hours_v, sumIf(metric_value, metric_key = 'zoom_meeting_hours') AS zoom_meeting_hours_v, sumIf(metric_value, metric_key = 'teams_meetings') AS teams_meetings_v, sumIf(metric_value, metric_key = 'zoom_meetings') AS zoom_meetings_v, sumIf(metric_value, metric_key = 'meeting_free') AS meeting_free_v, sumIf(metric_value, metric_key = 'm365_teams_chats') AS m365_teams_chats_v, sumIf(metric_value, metric_key = 'slack_messages_sent') AS slack_messages_sent_v, sumIf(metric_value, metric_key = 'slack_channel_posts') AS slack_channel_posts_v, sumIf(metric_value, metric_key = 'slack_active_days') AS slack_active_days_v, sumIf(metric_value, metric_key = 'm365_files_shared_internal') AS m365_files_shared_internal_v, sumIf(metric_value, metric_key = 'm365_files_shared_external') AS m365_files_shared_external_v, sumIf(metric_value, metric_key = 'm365_files_engaged') AS m365_files_engaged_v, sumIf(metric_value, metric_key = 'm365_active_days') AS m365_active_days_v, if(sumIf(metric_value, metric_key = 'slack_active_days') > 0, round(sumIf(metric_value, metric_key = 'slack_messages_sent') / sumIf(metric_value, metric_key = 'slack_active_days'), 1), CAST(NULL AS Nullable(Float64))) AS slack_msgs_per_active_day_v, if(sumIf(metric_value, metric_key = 'slack_messages_sent') > 0, round(toFloat64(100) * greatest(toFloat64(0), sumIf(metric_value, metric_key = 'slack_messages_sent') - sumIf(metric_value, metric_key = 'slack_channel_posts')) / sumIf(metric_value, metric_key = 'slack_messages_sent'), 1), CAST(NULL AS Nullable(Float64))) AS slack_dm_ratio_v FROM insight.collab_bullet_rows GROUP BY person_id) pp ARRAY JOIN [('m365_emails_sent', m365_emails_sent_v), ('m365_emails_received', m365_emails_received_v), ('m365_emails_read', m365_emails_read_v), ('meeting_hours', meeting_hours_v), ('meetings_count', meetings_count_v), ('teams_meeting_hours', teams_meeting_hours_v), ('zoom_meeting_hours', zoom_meeting_hours_v), ('teams_meetings', teams_meetings_v), ('zoom_meetings', zoom_meetings_v), ('meeting_free', meeting_free_v), ('m365_teams_chats', m365_teams_chats_v), ('slack_messages_sent', slack_messages_sent_v), ('slack_channel_posts', slack_channel_posts_v), ('slack_active_days', slack_active_days_v), ('m365_files_shared_internal', m365_files_shared_internal_v), ('m365_files_shared_external', m365_files_shared_external_v), ('m365_files_engaged', m365_files_engaged_v), ('m365_active_days', m365_active_days_v), ('slack_msgs_per_active_day', slack_msgs_per_active_day_v), ('slack_dm_ratio', slack_dm_ratio_v)] AS kv";

const GIT_QR: &str = "SELECT person_id, kv.1 AS metric_key, kv.2 AS value FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, sumIf(metric_value, metric_key = 'commits') AS commits, sumIf(metric_value, metric_key = 'loc') AS loc, sumIf(metric_value, metric_key = 'clean_loc') AS clean_loc, sumIf(metric_value, metric_key = 'prs_created') AS prs_created, sumIf(metric_value, metric_key = 'prs_merged') AS prs_merged, countIf(metric_key = 'commits' AND metric_value > 0) AS active_days, quantileExactIf(0.5)(metric_value, metric_key = 'pr_cycle_time_h') AS pr_cycle_time_h, quantileExactIf(0.5)(metric_value, metric_key = 'pr_size') AS pr_size FROM insight.git_bullet_rows GROUP BY person_id) pp ARRAY JOIN [('commits', toFloat64(commits)), ('prs_created', toFloat64(prs_created)), ('prs_merged', toFloat64(prs_merged)), ('clean_loc', toFloat64(clean_loc)), ('pr_cycle_time_h', pr_cycle_time_h), ('pr_size', pr_size), ('merge_rate', if(prs_created > 0, prs_merged * 100.0 / prs_created, NULL)), ('lines_per_commit', if(commits > 0, loc * 1.0 / commits, NULL)), ('commits_per_active_day', if(active_days > 0, commits * 1.0 / active_days, NULL))] AS kv";

/// `Member PRs Merged` (`…0043`): per-person PRs merged from the canonical
/// weekly silver. The inner subquery normalizes `person_key`/`week` to
/// `person_id`/`metric_date` so the handler's date-range filter binds; the
/// outer sums each person's weekly rows over the selected period.
const MEMBER_PRS_QR: &str = "SELECT person_id, sum(prs_merged) AS prs_merged FROM (SELECT person_key AS person_id, week AS metric_date, prs_merged FROM silver.mtr_git_person_weekly) GROUP BY person_id";

/// Per-person AI wide aggregate, copied verbatim from
/// `m20260606_000001_dept_metric_distributions::ai_wide_aggregate_pp`.
fn ai_wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         sumIf(metric_value, metric_key = 'cursor_completions') AS cursor_completions_v, \
         sumIf(metric_value, metric_key = 'cursor_agents') AS cursor_agents_v, \
         sumIf(metric_value, metric_key = 'cursor_lines') AS cursor_lines_v, \
         sumIf(metric_value, metric_key = 'cc_sessions') AS cc_sessions_v, \
         sumIf(metric_value, metric_key = 'cc_lines') AS cc_lines_v, \
         sumIf(metric_value, metric_key = 'cc_tool_accept') AS cc_tool_accept_v, \
         sumIf(metric_value, metric_key = 'team_ai_loc') AS team_ai_loc_v, \
         if(sumIf(metric_value, metric_key = 'cursor_offered') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'cursor_completions') \
                  / sumIf(metric_value, metric_key = 'cursor_offered'), 1), \
            CAST(NULL AS Nullable(Float64))) AS cursor_acceptance_v, \
         if(sumIf(metric_value, metric_key = 'cc_offered') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'cc_tool_accept') \
                  / sumIf(metric_value, metric_key = 'cc_offered'), 1), \
            CAST(NULL AS Nullable(Float64))) AS cc_tool_acceptance_v, \
         if(sumIf(metric_value, metric_key = 'cursor_total_lines') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'cursor_lines') \
                  / sumIf(metric_value, metric_key = 'cursor_total_lines'), 1), \
            CAST(NULL AS Nullable(Float64))) AS ai_loc_share2_v \
     FROM insight.ai_bullet_rows \
     GROUP BY person_id"
}

/// ARRAY JOIN unpivot for the distributable AI keys, copied verbatim from
/// `m20260606_000001_dept_metric_distributions::ai_array_join_kv`.
fn ai_array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('cursor_completions', cursor_completions_v), \
         ('cursor_agents',      cursor_agents_v), \
         ('cursor_lines',       cursor_lines_v), \
         ('cc_sessions',        cc_sessions_v), \
         ('cc_lines',           cc_lines_v), \
         ('cc_tool_accept',     cc_tool_accept_v), \
         ('team_ai_loc',        team_ai_loc_v), \
         ('cursor_acceptance',  cursor_acceptance_v), \
         ('cc_tool_acceptance', cc_tool_acceptance_v), \
         ('ai_loc_share2',      ai_loc_share2_v) \
     ] AS kv"
}

fn ai_member_values_query() -> String {
    format!(
        "SELECT person_id, kv.1 AS metric_key, kv.2 AS value \
         FROM ({pp}) pp \
         {kv}",
        pp = ai_wide_aggregate_pp(),
        kv = ai_array_join_kv(),
    )
}

const SEEDS: &[(&str, &str, &str)] = &[
    (
        DELIVERY_HEX,
        "Team Member Values — Task Delivery",
        "Per-person task-delivery metric values for a roster (person_id IN). Long rows (person_id, metric_key, value); no cohort.",
    ),
    (
        COLLAB_HEX,
        "Team Member Values — Collaboration",
        "Per-person collaboration metric values for a roster (person_id IN). Long rows (person_id, metric_key, value); no cohort.",
    ),
    (
        GIT_HEX,
        "Team Member Values — Git",
        "Per-person git metric values (incl. prs_merged) for a roster (person_id IN). Long rows (person_id, metric_key, value); no cohort.",
    ),
    (
        MEMBER_PRS_HEX,
        "Member PRs Merged",
        "Per-person PRs merged for a roster (person_id IN), period-bounded, from silver.mtr_git_person_weekly.",
    ),
    (
        MEMBER_VALUES_AI_HEX,
        "Team Member Values — AI",
        "Per-person AI metric values for a roster (person_id IN). Long rows (person_id, metric_key, value); no cohort. Distributable AI keys only (active-counter flags and NULL placeholders excluded).",
    ),
];

fn qr_for(hex: &str) -> String {
    match hex {
        DELIVERY_HEX => DELIVERY_QR.to_owned(),
        COLLAB_HEX => COLLAB_QR.to_owned(),
        GIT_HEX => GIT_QR.to_owned(),
        MEMBER_PRS_HEX => MEMBER_PRS_QR.to_owned(),
        MEMBER_VALUES_AI_HEX => ai_member_values_query(),
        _ => unreachable!(),
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex, name, description) in SEEDS {
            db.execute_unprepared(&format!(
                "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
                 VALUES (UNHEX('{hex}'), UNHEX('{ZERO_TENANT}'), '{name}', '{description}', '{qr}', 1) \
                 ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref), is_enabled=1",
                qr = qr_for(hex).replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex, _, _) in SEEDS {
            db.execute_unprepared(&format!("DELETE FROM metrics WHERE id = UNHEX('{hex}')"))
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_rows_per_person_no_cohort() {
        for qr in [DELIVERY_QR, COLLAB_QR, GIT_QR] {
            assert!(
                qr.starts_with("SELECT person_id, kv.1 AS metric_key, kv.2 AS value FROM ("),
                "must emit per-person long rows"
            );
            assert!(qr.contains("ARRAY JOIN ["), "must unpivot to long rows");
            assert!(qr.contains("GROUP BY person_id"), "per-person rollup");
            // No cohort join / distribution columns — the team view colors
            // client-side, so these stay value-only.
            for forbidden in [
                "LEFT JOIN",
                "company_median",
                "team_median",
                "_p25",
                "_p75",
                " AS median",
            ] {
                assert!(!qr.contains(forbidden), "must NOT contain {forbidden:?}");
            }
        }
    }

    #[test]
    fn each_reads_its_section_view() {
        assert!(DELIVERY_QR.contains("insight.task_delivery_bullet_rows"));
        assert!(COLLAB_QR.contains("insight.collab_bullet_rows"));
        assert!(GIT_QR.contains("insight.git_bullet_rows"));
    }

    #[test]
    fn heatmap_metric_keys_present() {
        assert!(DELIVERY_QR.contains("'mean_time_to_resolution'"));
        assert!(COLLAB_QR.contains("('meeting_hours', meeting_hours_v)"));
        assert!(GIT_QR.contains("('prs_merged', toFloat64(prs_merged))"));
    }

    #[test]
    fn member_prs_reads_weekly_silver() {
        assert!(MEMBER_PRS_QR.contains("FROM silver.mtr_git_person_weekly"));
        assert!(
            MEMBER_PRS_QR.contains("week AS metric_date"),
            "must normalize week for date-filter injection"
        );
        assert!(MEMBER_PRS_QR.contains("sum(prs_merged)"));
        assert!(MEMBER_PRS_QR.contains("GROUP BY person_id"));
        // Decoupled: must NOT read the IC dashboard view.
        assert!(!MEMBER_PRS_QR.contains("ic_kpis"));
    }

    #[test]
    fn ai_long_rows_per_person_no_cohort() {
        let qr = ai_member_values_query();
        assert!(
            qr.starts_with("SELECT person_id, kv.1 AS metric_key, kv.2 AS value FROM ("),
            "must emit per-person long rows"
        );
        assert!(qr.contains("ARRAY JOIN ["), "must unpivot to long rows");
        assert!(qr.contains("GROUP BY person_id"), "per-person rollup");
        assert!(qr.contains("insight.ai_bullet_rows"), "reads the AI source");
        for forbidden in ["LEFT JOIN", "team_median", "_p25", "_p75", " AS median"] {
            assert!(!qr.contains(forbidden), "must NOT contain {forbidden:?}");
        }
    }

    #[test]
    fn ai_excludes_active_counters_and_placeholders() {
        let qr = ai_member_values_query();
        for key in ["cursor_acceptance", "cc_tool_acceptance", "ai_loc_share2"] {
            assert!(
                qr.contains(&format!("'{key}'")),
                "missing distributable key {key}"
            );
        }
        for key in [
            "active_ai_members",
            "cursor_active",
            "cc_active",
            "chatgpt",
            "claude_web",
        ] {
            assert!(
                !qr.contains(&format!("('{key}'")),
                "non-distributable key {key} must be excluded"
            );
        }
    }
}
