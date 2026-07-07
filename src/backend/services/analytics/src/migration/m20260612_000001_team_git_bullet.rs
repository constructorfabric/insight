//! Seed a Team Bullet Git metric (`…0007`) so the team view's "Git output"
//! section matches the IC dashboard's, backed by the same department-blend
//! cohort the other team bullets use (`m20260606_000003`).
//!
//! There was no team-scope git bullet before: the git bullet existed only as
//! the IC metric `…0018` (per-person, department cohort, `any(c.*)`). The team
//! view's section set is being aligned to the IC set (Task delivery, Git
//! output, Collaboration, AI), so the team needs its own git bullet that:
//!   - reads the same `insight.git_bullet_rows` per-person aggregate as `…0018`
//!     (commits / `clean_loc` / prs / cycle-time / size + the computed ratios);
//!   - blends the members' DEPARTMENT cohorts via `avg(c.team_*)` joined on
//!     `org_unit_id` (the headcount-weighted expectation), with `n =
//!     count(p.v_period)` — identical in shape to the delivery/collab/ai team
//!     bullets after `m20260606_000003`.
//!
//! Roster scoping stays at the handler (`person_id IN (roster)`), so the outer
//! keeps `GROUP BY p.metric_key`; both leaves keep `GROUP BY person_id` for the
//! date-walker. Value is `avgIf(p.v_period, isNotNull(p.v_period))` (the git
//! `*If` convention), so empty git rows don't dilute the team average.
//!
//! `down()` deletes the metric (append-only seed).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ZERO_TENANT: &str = "00000000000000000000000000000000";
const TEAM_BULLET_GIT_HEX: &str = "00000000000000000001000000000007";

/// Per-person git aggregate, copied verbatim from `m20260604_000005`'s
/// `…0018` query (`insight.git_bullet_rows`, one row per person).
const GIT_PP: &str = "SELECT person_id, any(org_unit_id) AS org_unit_id, \
    sumIf(metric_value, metric_key = 'commits') AS commits, \
    sumIf(metric_value, metric_key = 'loc') AS loc, \
    sumIf(metric_value, metric_key = 'clean_loc') AS clean_loc, \
    sumIf(metric_value, metric_key = 'prs_created') AS prs_created, \
    sumIf(metric_value, metric_key = 'prs_merged') AS prs_merged, \
    countIf(metric_key = 'commits' AND metric_value > 0) AS active_days, \
    quantileExactIfOrNull(0.5)(metric_value, metric_key = 'pr_cycle_time_h') AS pr_cycle_time_h, \
    quantileExactIfOrNull(0.5)(metric_value, metric_key = 'pr_size') AS pr_size \
    FROM insight.git_bullet_rows GROUP BY person_id";

/// ARRAY JOIN unpivot for the git bullet, copied verbatim from `…0018`.
const GIT_KV: &str = "ARRAY JOIN [('commits', toFloat64(commits)), \
    ('prs_created', toFloat64(prs_created)), \
    ('prs_merged', toFloat64(prs_merged)), \
    ('clean_loc', toFloat64(clean_loc)), \
    ('pr_cycle_time_h', pr_cycle_time_h), \
    ('pr_size', pr_size), \
    ('merge_rate', if(prs_created > 0, prs_merged * 100.0 / prs_created, NULL)), \
    ('lines_per_commit', if(commits > 0, loc * 1.0 / commits, NULL)), \
    ('commits_per_active_day', if(active_days > 0, commits * 1.0 / active_days, NULL))] AS kv";

/// Team git bullet: per-person git aggregate joined to the DEPARTMENT cohort
/// on `org_unit_id`, blended via `avg(c.team_*)` (headcount-weighted over the
/// roster). Mirrors `m20260606_000003`'s blend, applied to the `…0018` git
/// aggregate.
fn team_git_query() -> String {
    format!(
        "SELECT p.metric_key AS metric_key, \
                avgIfOrNull(p.v_period, isNotNull(p.v_period)) AS value, \
                avg(c.team_median) AS median, \
                avg(c.team_min) AS range_min, \
                avg(c.team_max) AS range_max, \
                avg(c.team_p25) AS p25, \
                avg(c.team_p75) AS p75, \
                toFloat64(count(p.v_period)) AS n \
         FROM ( \
             SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({GIT_PP}) pp \
             {GIT_KV} \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, org_unit_id, \
                    quantileExactIfOrNull(0.5)(v_period, isNotNull(v_period)) AS team_median, \
                    minIf(v_period, isNotNull(v_period)) AS team_min, \
                    maxIf(v_period, isNotNull(v_period)) AS team_max, \
                    quantileExactIfOrNull(0.25)(v_period, isNotNull(v_period)) AS team_p25, \
                    quantileExactIfOrNull(0.75)(v_period, isNotNull(v_period)) AS team_p75, \
                    countIf(isNotNull(v_period)) AS team_n \
             FROM ( \
                 SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
                 FROM ({GIT_PP}) ppc \
                 {GIT_KV} \
             ) inner_c \
             GROUP BY metric_key, org_unit_id \
         ) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
         GROUP BY p.metric_key"
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
             VALUES (UNHEX('{TEAM_BULLET_GIT_HEX}'), UNHEX('{ZERO_TENANT}'), 'Team Bullet Git', \
             'Team git bullet for the team view Git output section: per-roster git aggregate (insight.git_bullet_rows) blended against the members'' department cohorts (avg(c.team_*)). Scope with person_id IN (roster).', \
             '{qr}', 1) \
             ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref), is_enabled=1",
            qr = team_git_query().replace('\'', "''"),
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "DELETE FROM metrics WHERE id = UNHEX('{TEAM_BULLET_GIT_HEX}')"
        ))
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blends_department_cohort_over_git_aggregate() {
        let q = team_git_query();
        // Department cohort joined on org_unit_id, blended via avg(c.*).
        assert!(q.contains("c.org_unit_id = p.org_unit_id"));
        for col in ["avg(c.team_median)", "avg(c.team_p25)", "avg(c.team_p75)"] {
            assert!(q.contains(col), "git team bullet must blend via `{col}`");
        }
        assert!(!q.contains("any(c.team_"), "must blend (avg), not any()");
        assert!(q.contains("toFloat64(count(p.v_period)) AS n"));
        // Git value convention: avgIfOrNull over non-null period values
        // (NULL — not NaN — on an empty set, so isNotNull-gated reads skip it).
        assert!(q.contains("avgIfOrNull(p.v_period, isNotNull(p.v_period)) AS value"));
    }

    #[test]
    fn emits_the_git_bullet_keys() {
        let q = team_git_query();
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
            assert!(q.contains(&format!("'{key}'")), "missing git key {key}");
        }
        assert_eq!(
            q.matches("insight.git_bullet_rows").count(),
            2,
            "both JOIN sides read git_bullet_rows"
        );
    }
}
