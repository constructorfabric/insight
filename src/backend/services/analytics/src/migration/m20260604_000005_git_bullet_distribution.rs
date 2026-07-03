//! Add cohort distribution stats (p25 / p75 / n) to the IC Bullet Git
//! `query_ref`, on top of the median/min/max already surfaced by
//! `m20260430_000001_update_git_bullet`.
//!
//! The FE distribution strip needs the interquartile range and the
//! cohort size alongside the existing median and full range. This
//! migration extends only the cohort (`c`) aggregate and the OUTER
//! SELECT with three additions; everything else is carried over
//! verbatim from the predecessor's `query_ref`:
//!
//!   1. `c` side (per `metric_key, org_unit_id` department cohort):
//!      `quantileExactIf(0.25)(v_period, isNotNull(v_period)) AS team_p25`,
//!      `quantileExactIf(0.75)(v_period, isNotNull(v_period)) AS team_p75`,
//!      `countIf(isNotNull(v_period)) AS team_n` — same `*If` family as
//!      the existing `team_median` / `team_min` / `team_max`.
//!   2. OUTER SELECT: `any(c.team_p25) AS p25`, `any(c.team_p75) AS p75`,
//!      `any(c.team_n) AS n`.
//!
//! IC-only: there is no Team Git variant. The cohort join stays the
//! department-scoped `c.org_unit_id = p.org_unit_id` — no supervisor
//! scope is introduced.
//!
//! `down()` restores the predecessor's current `query_ref` verbatim
//! (the value `OLD_QUERY_REF` below, which equals
//! `m20260430_000001`'s `NEW_QUERY_REF`).
//!
//! UUID matches the existing IC Bullet Git seed
//! (`00000000000000000001000000000018`); we update the `query_ref` only.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const IC_BULLET_GIT_HEX: &str = "00000000000000000001000000000018";

const NEW_QUERY_REF: &str = "SELECT p.metric_key AS metric_key, avgIf(p.v_period, isNotNull(p.v_period)) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max, any(c.team_p25) AS p25, any(c.team_p75) AS p75, any(c.team_n) AS n FROM (SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, sumIf(metric_value, metric_key = 'commits') AS commits, sumIf(metric_value, metric_key = 'loc') AS loc, sumIf(metric_value, metric_key = 'clean_loc') AS clean_loc, sumIf(metric_value, metric_key = 'prs_created') AS prs_created, sumIf(metric_value, metric_key = 'prs_merged') AS prs_merged, countIf(metric_key = 'commits' AND metric_value > 0) AS active_days, quantileExactIf(0.5)(metric_value, metric_key = 'pr_cycle_time_h') AS pr_cycle_time_h, quantileExactIf(0.5)(metric_value, metric_key = 'pr_size') AS pr_size FROM insight.git_bullet_rows GROUP BY person_id) ARRAY JOIN [('commits', toFloat64(commits)), ('prs_created', toFloat64(prs_created)), ('prs_merged', toFloat64(prs_merged)), ('clean_loc', toFloat64(clean_loc)), ('pr_cycle_time_h', pr_cycle_time_h), ('pr_size', pr_size), ('merge_rate', if(prs_created > 0, prs_merged * 100.0 / prs_created, NULL)), ('lines_per_commit', if(commits > 0, loc * 1.0 / commits, NULL)), ('commits_per_active_day', if(active_days > 0, commits * 1.0 / active_days, NULL))] AS kv) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExactIf(0.5)(v_period, isNotNull(v_period)) AS team_median, minIf(v_period, isNotNull(v_period)) AS team_min, maxIf(v_period, isNotNull(v_period)) AS team_max, quantileExactIf(0.25)(v_period, isNotNull(v_period)) AS team_p25, quantileExactIf(0.75)(v_period, isNotNull(v_period)) AS team_p75, countIf(isNotNull(v_period)) AS team_n FROM (SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, sumIf(metric_value, metric_key = 'commits') AS commits, sumIf(metric_value, metric_key = 'loc') AS loc, sumIf(metric_value, metric_key = 'clean_loc') AS clean_loc, sumIf(metric_value, metric_key = 'prs_created') AS prs_created, sumIf(metric_value, metric_key = 'prs_merged') AS prs_merged, countIf(metric_key = 'commits' AND metric_value > 0) AS active_days, quantileExactIf(0.5)(metric_value, metric_key = 'pr_cycle_time_h') AS pr_cycle_time_h, quantileExactIf(0.5)(metric_value, metric_key = 'pr_size') AS pr_size FROM insight.git_bullet_rows GROUP BY person_id) ARRAY JOIN [('commits', toFloat64(commits)), ('prs_created', toFloat64(prs_created)), ('prs_merged', toFloat64(prs_merged)), ('clean_loc', toFloat64(clean_loc)), ('pr_cycle_time_h', pr_cycle_time_h), ('pr_size', pr_size), ('merge_rate', if(prs_created > 0, prs_merged * 100.0 / prs_created, NULL)), ('lines_per_commit', if(commits > 0, loc * 1.0 / commits, NULL)), ('commits_per_active_day', if(active_days > 0, commits * 1.0 / active_days, NULL))] AS kv) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key";

const OLD_QUERY_REF: &str = "SELECT p.metric_key AS metric_key, avgIf(p.v_period, isNotNull(p.v_period)) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, sumIf(metric_value, metric_key = 'commits') AS commits, sumIf(metric_value, metric_key = 'loc') AS loc, sumIf(metric_value, metric_key = 'clean_loc') AS clean_loc, sumIf(metric_value, metric_key = 'prs_created') AS prs_created, sumIf(metric_value, metric_key = 'prs_merged') AS prs_merged, countIf(metric_key = 'commits' AND metric_value > 0) AS active_days, quantileExactIf(0.5)(metric_value, metric_key = 'pr_cycle_time_h') AS pr_cycle_time_h, quantileExactIf(0.5)(metric_value, metric_key = 'pr_size') AS pr_size FROM insight.git_bullet_rows GROUP BY person_id) ARRAY JOIN [('commits', toFloat64(commits)), ('prs_created', toFloat64(prs_created)), ('prs_merged', toFloat64(prs_merged)), ('clean_loc', toFloat64(clean_loc)), ('pr_cycle_time_h', pr_cycle_time_h), ('pr_size', pr_size), ('merge_rate', if(prs_created > 0, prs_merged * 100.0 / prs_created, NULL)), ('lines_per_commit', if(commits > 0, loc * 1.0 / commits, NULL)), ('commits_per_active_day', if(active_days > 0, commits * 1.0 / active_days, NULL))] AS kv) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExactIf(0.5)(v_period, isNotNull(v_period)) AS team_median, minIf(v_period, isNotNull(v_period)) AS team_min, maxIf(v_period, isNotNull(v_period)) AS team_max FROM (SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, sumIf(metric_value, metric_key = 'commits') AS commits, sumIf(metric_value, metric_key = 'loc') AS loc, sumIf(metric_value, metric_key = 'clean_loc') AS clean_loc, sumIf(metric_value, metric_key = 'prs_created') AS prs_created, sumIf(metric_value, metric_key = 'prs_merged') AS prs_merged, countIf(metric_key = 'commits' AND metric_value > 0) AS active_days, quantileExactIf(0.5)(metric_value, metric_key = 'pr_cycle_time_h') AS pr_cycle_time_h, quantileExactIf(0.5)(metric_value, metric_key = 'pr_size') AS pr_size FROM insight.git_bullet_rows GROUP BY person_id) ARRAY JOIN [('commits', toFloat64(commits)), ('prs_created', toFloat64(prs_created)), ('prs_merged', toFloat64(prs_merged)), ('clean_loc', toFloat64(clean_loc)), ('pr_cycle_time_h', pr_cycle_time_h), ('pr_size', pr_size), ('merge_rate', if(prs_created > 0, prs_merged * 100.0 / prs_created, NULL)), ('lines_per_commit', if(commits > 0, loc * 1.0 / commits, NULL)), ('commits_per_active_day', if(active_days > 0, commits * 1.0 / active_days, NULL))] AS kv) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key";

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_query_adds_distribution_stats() {
        let q = NEW_QUERY_REF;

        for token in ["team_p25", "team_p75", "team_n", "AS p25", "AS p75", "AS n"] {
            assert!(q.contains(token), "NEW_QUERY_REF must contain {token:?}");
        }

        assert!(
            q.contains("quantileExactIf(0.25)(v_period, isNotNull(v_period)) AS team_p25"),
            "NEW_QUERY_REF must compute team_p25 via quantileExactIf(0.25) on the cohort side"
        );
        assert!(
            q.contains("quantileExactIf(0.75)(v_period, isNotNull(v_period)) AS team_p75"),
            "NEW_QUERY_REF must compute team_p75 via quantileExactIf(0.75) on the cohort side"
        );
        assert!(
            q.contains("countIf(isNotNull(v_period)) AS team_n"),
            "NEW_QUERY_REF must compute the cohort size team_n via countIf(isNotNull(v_period))"
        );
    }

    #[test]
    fn new_query_keeps_department_cohort_join() {
        assert!(
            NEW_QUERY_REF.contains("c.org_unit_id = p.org_unit_id"),
            "NEW_QUERY_REF must retain the department cohort join"
        );
    }

    #[test]
    fn new_query_is_ic_only_no_supervisor_scope() {
        assert!(
            !NEW_QUERY_REF.contains("supervisor_email"),
            "NEW_QUERY_REF is IC-only and must not introduce supervisor_email scope"
        );
    }

    #[test]
    fn new_query_targets_git_bullet_metric_keys() {
        for key in [
            "commits",
            "prs_created",
            "prs_merged",
            "clean_loc",
            "pr_cycle_time_h",
            "pr_size",
            "merge_rate",
            "lines_per_commit",
            "commits_per_active_day",
        ] {
            assert!(
                NEW_QUERY_REF.contains(&format!("'{key}'")),
                "NEW_QUERY_REF must emit metric_key {key:?}"
            );
        }
    }

    #[test]
    fn down_restores_predecessor_query() {
        assert!(
            !OLD_QUERY_REF.contains("team_p25")
                && !OLD_QUERY_REF.contains("team_p75")
                && !OLD_QUERY_REF.contains("team_n"),
            "OLD_QUERY_REF (down target) must be the predecessor query without distribution stats"
        );
        assert!(
            OLD_QUERY_REF.contains("c.org_unit_id = p.org_unit_id"),
            "OLD_QUERY_REF must retain the department cohort join"
        );
    }
}
