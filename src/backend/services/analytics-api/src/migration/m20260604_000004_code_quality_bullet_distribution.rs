//! Add cohort distribution (p25/p75 + size `n`) to the Team Bullet Code
//! Quality `query_ref`, on top of the wide-aggregate + `ARRAY JOIN` shape
//! introduced by `m20260520_000001_code_quality_bullet_rewrite`.
//!
//! The company-side aggregation gains `company_p25` / `company_p75`
//! (`quantileExact(0.25|0.75)`) and `company_n` (`count(v_period)`),
//! surfaced on the outer SELECT as `p25`/`p75`/`n`, so the FE can render
//! the interquartile band and the cohort size. Over the all-NULL
//! `ComingSoon` columns (`prs_per_dev` / `pr_cycle_time` /
//! `build_success`) the quantiles yield NULL and the count yields 0 — the
//! honest-NULL → `ComingSoon` contract is preserved. The cohort stays
//! company-wide.
//!
//! Team-only: there is no `IC_BULLET_CODE_QUALITY` metric in the seed
//! (`m20260422_000001_seed_metrics`) — only the Team-scope `…04` exists.
//!
//! Team value scoping is done by the handler's `person_id IN (roster)`
//! filter, so the query keeps the original `GROUP BY metric_key` shape —
//! no supervisor join. Both leaves still `GROUP BY person_id`, so
//! `inject_date_filter_into_subqueries` injects the metric_date range
//! before each GROUP BY exactly as before.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_CODE_QUALITY_ID: &str = "00000000000000000001000000000004";

/// Wide-aggregate block: one row per `person_id` with every FE-visible
/// `metric_key` materialized in its own column. Used by both the
/// per-person (`p`) side and the company-wide range/quartile
/// aggregation (`inner_c`). Copied verbatim from `m20260520_000001`.
///   - `bugs_fixed_v`: `sumIf` period sum (the one real metric).
///   - `prs_per_dev_v` / `pr_cycle_time_v` / `build_success_v`:
///     hardcoded NULL — the view no longer emits these (no ingestion
///     source). The honest-NULL contract renders them as `ComingSoon`.
fn wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         sumIf(metric_value, metric_key = 'bugs_fixed') AS bugs_fixed_v, \
         CAST(NULL AS Nullable(Float64)) AS prs_per_dev_v, \
         CAST(NULL AS Nullable(Float64)) AS pr_cycle_time_v, \
         CAST(NULL AS Nullable(Float64)) AS build_success_v \
     FROM insight.code_quality_bullet_rows \
     GROUP BY person_id"
}

/// `ARRAY JOIN` unpivot: 4 wide columns → 4 long rows per person.
/// 1 view-emitted key + 3 `ComingSoon` hardcoded-NULL keys = 4
/// FE-visible `metric_key`s (matches the predecessor's response set).
fn array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('bugs_fixed',    bugs_fixed_v), \
         ('prs_per_dev',   prs_per_dev_v), \
         ('pr_cycle_time', pr_cycle_time_v), \
         ('build_success', build_success_v) \
     ] AS kv"
}

fn team_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    format!(
        "SELECT p.metric_key AS metric_key, \
                avg(p.v_period) AS value, \
                any(c.company_median) AS median, \
                any(c.company_min) AS range_min, \
                any(c.company_max) AS range_max, \
                any(c.company_p25) AS p25, \
                any(c.company_p75) AS p75, \
                any(c.company_n) AS n \
         FROM ( \
             SELECT person_id, org_unit_id, \
                    kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({pp}) pp \
             {kv} \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, \
                    quantileExact(0.5)(v_period) AS company_median, \
                    min(v_period) AS company_min, \
                    max(v_period) AS company_max, \
                    quantileExact(0.25)(v_period) AS company_p25, \
                    quantileExact(0.75)(v_period) AS company_p75, \
                    count(v_period) AS company_n \
             FROM ( \
                 SELECT kv.1 AS metric_key, kv.2 AS v_period \
                 FROM ({pp}) ppc \
                 {kv} \
             ) inner_c \
             GROUP BY metric_key \
         ) c ON c.metric_key = p.metric_key \
         GROUP BY p.metric_key"
    )
}

/// The query string installed by the predecessor
/// `m20260520_000001_code_quality_bullet_rewrite`. `down()` restores
/// this verbatim so a rollback returns the catalog to its prior state.
fn old_team_query() -> String {
    "SELECT p.metric_key AS metric_key, \
                avg(p.v_period) AS value, \
                any(c.company_median) AS median, \
                any(c.company_min) AS range_min, \
                any(c.company_max) AS range_max \
         FROM ( \
             SELECT person_id, org_unit_id, \
                    kv.1 AS metric_key, kv.2 AS v_period \
             FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, \
         sumIf(metric_value, metric_key = 'bugs_fixed') AS bugs_fixed_v, \
         CAST(NULL AS Nullable(Float64)) AS prs_per_dev_v, \
         CAST(NULL AS Nullable(Float64)) AS pr_cycle_time_v, \
         CAST(NULL AS Nullable(Float64)) AS build_success_v \
     FROM insight.code_quality_bullet_rows \
     GROUP BY person_id) pp \
             ARRAY JOIN [ \
         ('bugs_fixed',    bugs_fixed_v), \
         ('prs_per_dev',   prs_per_dev_v), \
         ('pr_cycle_time', pr_cycle_time_v), \
         ('build_success', build_success_v) \
     ] AS kv \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, \
                    quantileExact(0.5)(v_period) AS company_median, \
                    min(v_period) AS company_min, \
                    max(v_period) AS company_max \
             FROM ( \
                 SELECT kv.1 AS metric_key, kv.2 AS v_period \
                 FROM (SELECT person_id, any(org_unit_id) AS org_unit_id, \
         sumIf(metric_value, metric_key = 'bugs_fixed') AS bugs_fixed_v, \
         CAST(NULL AS Nullable(Float64)) AS prs_per_dev_v, \
         CAST(NULL AS Nullable(Float64)) AS pr_cycle_time_v, \
         CAST(NULL AS Nullable(Float64)) AS build_success_v \
     FROM insight.code_quality_bullet_rows \
     GROUP BY person_id) ppc \
                 ARRAY JOIN [ \
         ('bugs_fixed',    bugs_fixed_v), \
         ('prs_per_dev',   prs_per_dev_v), \
         ('pr_cycle_time', pr_cycle_time_v), \
         ('build_success', build_success_v) \
     ] AS kv \
             ) inner_c \
             GROUP BY metric_key \
         ) c ON c.metric_key = p.metric_key \
         GROUP BY p.metric_key"
        .to_string()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{TEAM_BULLET_CODE_QUALITY_ID}')",
            qr = team_query().replace('\'', "''"),
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{TEAM_BULLET_CODE_QUALITY_ID}')",
            qr = old_team_query().replace('\'', "''"),
        ))
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // String-contains tests — same rationale as the prior bullet
    // rewrites in this series. Goal: catch typo regressions that would
    // silently aggregate to NULL, missing ComingSoon hardcodes, and
    // walker-shape drift.

    /// Every FE-visible `metric_key` the bullet section emits must
    /// appear as an `('X', X_v)` entry in the ARRAY JOIN unpivot.
    /// 1 view-emitted + 3 `ComingSoon` hardcoded = 4 total.
    const EXPECTED_METRIC_KEYS: &[&str] = &[
        "bugs_fixed",
        "prs_per_dev",
        "pr_cycle_time",
        "build_success",
    ];

    /// The single raw `metric_key` the view emits that `query_ref`
    /// reads via `sumIf`. A typo here = silent NULL.
    const EXPECTED_RAW_KEYS_READ_BY_QUERY: &[&str] = &["bugs_fixed"];

    /// `metric_key`s that must NOT be read via `sumIf` — the view no
    /// longer emits them, so a `metric_key = 'X'` read would silently
    /// aggregate to NULL.
    const FORBIDDEN_RAW_KEY_READS: &[&str] = &["prs_per_dev", "pr_cycle_time", "build_success"];

    fn assert_query_shape(query: &str, label: &str) {
        let table_refs = query.matches("insight.code_quality_bullet_rows").count();
        assert_eq!(
            table_refs, 2,
            "{label}: expected 2 references to `insight.code_quality_bullet_rows`, got {table_refs}"
        );

        for key in EXPECTED_METRIC_KEYS {
            let literal = format!("'{key}'");
            assert!(
                query.contains(&literal),
                "{label}: missing FE-visible metric_key literal {literal} in ARRAY JOIN unpivot"
            );
        }

        for key in EXPECTED_RAW_KEYS_READ_BY_QUERY {
            let read = format!("metric_key = '{key}'");
            assert!(
                query.contains(&read),
                "{label}: missing read of raw metric_key {key} in wide-aggregate"
            );
        }

        for key in FORBIDDEN_RAW_KEY_READS {
            let read = format!("metric_key = '{key}'");
            assert!(
                !query.contains(&read),
                "{label}: dropped metric_key {key} must not be read from the view (it's no longer emitted)"
            );
        }

        // ComingSoon keys must be hardcoded NULL columns in the
        // wide-aggregate (the honest-NULL contract).
        for key in ["prs_per_dev_v", "pr_cycle_time_v", "build_success_v"] {
            assert!(
                query.contains(&format!("CAST(NULL AS Nullable(Float64)) AS {key}")),
                "{label}: ComingSoon key alias {key} must be hardcoded NULL via `CAST(NULL AS Nullable(Float64)) AS {key}`"
            );
        }
    }

    #[test]
    fn team_query_shape() {
        let q = team_query();
        assert_query_shape(&q, "team_query");
        // Team-scope: company-wide range (no IC variant exists for this
        // section, so no team_* labels should appear).
        assert!(
            q.contains("company_median") && q.contains("company_min") && q.contains("company_max"),
            "team_query must expose company_* range, got:\n{q}"
        );
        assert!(
            !q.contains("team_median"),
            "team_query must NOT use team_median (no IC variant for code quality)"
        );
        assert!(
            q.contains("ON c.metric_key = p.metric_key"),
            "team_query JOIN must be on metric_key alone"
        );

        // Roster scoping happens at the handler (`person_id IN (...)`), so
        // the query stays company-wide + groups by metric_key only — no
        // supervisor join, no supervisor_email column.
        assert!(
            q.contains("GROUP BY p.metric_key"),
            "team_query outer GROUP BY must be metric_key, got:\n{q}"
        );
        assert!(
            !q.contains("supervisor_email"),
            "team_query must NOT reference supervisor_email (roster scope is at the handler)"
        );
        assert!(
            !q.contains("insight.people"),
            "team_query must NOT join insight.people"
        );
    }

    /// Cohort quartiles + size: the company-side aggregation must expose
    /// `company_p25` / `company_p75` (`quantileExact(0.25|0.75)`) and
    /// `company_n` (`count(v_period)`), surfaced on the outer SELECT as
    /// `p25` / `p75` / `n`.
    #[test]
    fn team_query_exposes_cohort_distribution() {
        let q = team_query();
        for agg in [
            "quantileExact(0.25)(v_period) AS company_p25",
            "quantileExact(0.75)(v_period) AS company_p75",
            "count(v_period) AS company_n",
        ] {
            assert!(
                q.contains(agg),
                "team_query company aggregation must contain `{agg}`, got:\n{q}"
            );
        }
        for col in [
            "any(c.company_p25) AS p25",
            "any(c.company_p75) AS p75",
            "any(c.company_n) AS n",
        ] {
            assert!(
                q.contains(col),
                "team_query outer SELECT must surface `{col}`, got:\n{q}"
            );
        }
    }

    /// Roster scoping happens at the handler (`person_id IN (...)`), so
    /// the query is NOT supervisor-scoped: no people-join, no
    /// `supervisor_email`, and both leaves keep a bare `GROUP BY
    /// person_id`.
    #[test]
    fn team_query_is_not_supervisor_scoped() {
        let q = team_query();
        assert!(
            !q.contains("supervisor_email"),
            "team_query must NOT reference supervisor_email (roster scope is at the handler), got:\n{q}"
        );
        assert!(
            !q.contains("insight.people"),
            "team_query must NOT join insight.people, got:\n{q}"
        );
        assert!(
            q.contains("GROUP BY p.metric_key"),
            "outer aggregate must group by metric_key alone, got:\n{q}"
        );

        // Both leaves keep a bare `GROUP BY person_id` (p and inner_c);
        // neither uses the people-join `r.person_id` spelling.
        assert_eq!(
            q.matches("GROUP BY person_id").count(),
            2,
            "both leaves must keep the bare `GROUP BY person_id`, got:\n{q}"
        );
        assert_eq!(
            q.matches("GROUP BY r.person_id").count(),
            0,
            "no leaf should use `GROUP BY r.person_id` (no people-join), got:\n{q}"
        );
    }

    /// `down()` must restore the predecessor query verbatim — same
    /// `metric_key`-only `GROUP BY`, no supervisor scope, no quartiles.
    #[test]
    fn old_query_is_predecessor_shape() {
        let old = old_team_query();
        assert_query_shape(&old, "old_team_query");
        assert!(
            old.contains("GROUP BY p.metric_key") && !old.contains("p.supervisor_email"),
            "old_team_query must be the predecessor metric_key-only shape, got:\n{old}"
        );
        assert!(
            !old.contains("company_p25")
                && !old.contains("company_p75")
                && !old.contains("company_n"),
            "old_team_query must NOT contain the new cohort-distribution columns, got:\n{old}"
        );
        assert!(
            !old.contains("LEFT JOIN insight.people"),
            "old_team_query must NOT people-join (predecessor had no supervisor scope), got:\n{old}"
        );
    }
}
