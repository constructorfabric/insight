//! Replace the IC Bullet Git `query_ref` to surface the expanded `git_output` set.
//!
//! Pairs with ingestion migration `20260430000000_git-bullet-expand.sql`, which
//! widens `insight.git_bullet_rows` from a single key (`commits`) to the
//! counters and per-event distributions needed by the IC dashboard.
//!
//! New query pivots the daily/per-event rows per (person, period) into the
//! 9 `metric_keys` the FE expects: 6 surfaced from the view directly
//! (`commits`, `prs_created`, `prs_merged`, `clean_loc`, `pr_size`,
//! `pr_cycle_time_h`) and 3 derived ratios computed from counter sums
//! per person (`merge_rate`, `lines_per_commit`, `commits_per_active_day`).
//!
//! Ratios are computed at the per-person aggregation step (Σ num / Σ den
//! over the period), not as averages of daily ratios — period totals are
//! the only mathematically correct definition for these.
//!
//! `pr_review_time` is intentionally not emitted; see the ingestion
//! migration's header for rationale.
//!
//! The date filter pushed by analytics-api (`metric_date >= … AND < …`) is
//! injected into each innermost `SELECT … FROM insight.git_bullet_rows
//! GROUP BY person_id` subquery by `inject_where_into_first_subquery` in
//! `handlers.rs`, so the period scope still applies cleanly despite the
//! more complex shape.
//!
//! UUID matches the existing IC Bullet Git seed
//! (`00000000000000000001000000000018`); we update the `query_ref` only.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const IC_BULLET_GIT_HEX: &str = "00000000000000000001000000000018";

const NEW_QUERY_REF: &str = "SELECT p.metric_key AS metric_key, avgIf(p.v_period, isNotNull(p.v_period)) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, sumIf(metric_value, metric_key = 'commits') AS commits, sumIf(metric_value, metric_key = 'loc') AS loc, sumIf(metric_value, metric_key = 'clean_loc') AS clean_loc, sumIf(metric_value, metric_key = 'prs_created') AS prs_created, sumIf(metric_value, metric_key = 'prs_merged') AS prs_merged, countIf(metric_key = 'commits' AND metric_value > 0) AS active_days, quantileExactIf(0.5)(metric_value, metric_key = 'pr_cycle_time_h') AS pr_cycle_time_h, quantileExactIf(0.5)(metric_value, metric_key = 'pr_size') AS pr_size FROM insight.git_bullet_rows GROUP BY person_id) ARRAY JOIN [('commits', toFloat64(commits)), ('prs_created', toFloat64(prs_created)), ('prs_merged', toFloat64(prs_merged)), ('clean_loc', toFloat64(clean_loc)), ('pr_cycle_time_h', pr_cycle_time_h), ('pr_size', pr_size), ('merge_rate', if(prs_created > 0, prs_merged * 100.0 / prs_created, NULL)), ('lines_per_commit', if(commits > 0, loc * 1.0 / commits, NULL)), ('commits_per_active_day', if(active_days > 0, commits * 1.0 / active_days, NULL))] AS kv) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExactIf(0.5)(v_period, isNotNull(v_period)) AS team_median, minIf(v_period, isNotNull(v_period)) AS team_min, maxIf(v_period, isNotNull(v_period)) AS team_max FROM (SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, sumIf(metric_value, metric_key = 'commits') AS commits, sumIf(metric_value, metric_key = 'loc') AS loc, sumIf(metric_value, metric_key = 'clean_loc') AS clean_loc, sumIf(metric_value, metric_key = 'prs_created') AS prs_created, sumIf(metric_value, metric_key = 'prs_merged') AS prs_merged, countIf(metric_key = 'commits' AND metric_value > 0) AS active_days, quantileExactIf(0.5)(metric_value, metric_key = 'pr_cycle_time_h') AS pr_cycle_time_h, quantileExactIf(0.5)(metric_value, metric_key = 'pr_size') AS pr_size FROM insight.git_bullet_rows GROUP BY person_id) ARRAY JOIN [('commits', toFloat64(commits)), ('prs_created', toFloat64(prs_created)), ('prs_merged', toFloat64(prs_merged)), ('clean_loc', toFloat64(clean_loc)), ('pr_cycle_time_h', pr_cycle_time_h), ('pr_size', pr_size), ('merge_rate', if(prs_created > 0, prs_merged * 100.0 / prs_created, NULL)), ('lines_per_commit', if(commits > 0, loc * 1.0 / commits, NULL)), ('commits_per_active_day', if(active_days > 0, commits * 1.0 / active_days, NULL))] AS kv) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key";

const OLD_QUERY_REF: &str = "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, sum(metric_value) AS v_period FROM insight.git_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExact(0.5)(v_period) AS team_median, min(v_period) AS team_min, max(v_period) AS team_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, sum(metric_value) AS v_period FROM insight.git_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{IC_BULLET_GIT_HEX}')",
            qr = NEW_QUERY_REF.replace('\'', "''"),
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{IC_BULLET_GIT_HEX}')",
            qr = OLD_QUERY_REF.replace('\'', "''"),
        ))
        .await?;
        Ok(())
    }
}
