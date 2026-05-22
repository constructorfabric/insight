//! Rewrite the Team Bullet Code Quality `query_ref` to consume the new
//! `code_quality_bullet_rows` shape (issue #433 §4.1, §4.5).
//!
//! Pairs with ingestion migration
//! `20260520000000_code-quality-bullet-rewrite.sql`, which drops 3
//! `ComingSoon` NULL-only emissions from the view and keeps just the
//! one real metric (`bugs_fixed`). This `query_ref` reconstructs the
//! 4 FE-visible `metric_key`s using `sumIf` for `bugs_fixed` and
//! hardcoded `NULL` columns for the 3 `ComingSoon` keys.
//!
//! Structural change (mirrors PR #478 / #480 / #490):
//!   - Replaced `multiIf(metric_key=X, dispatch)` inner with a
//!     wide-aggregate per `metric_key` + `ARRAY JOIN` unpivot back to
//!     long format. Outer is a plain `avg(p.v_period)` — this section
//!     has no active counters and no daily ratios, so no `multiIf`
//!     dispatch is needed at the outer level either.
//!
//! `ComingSoon` audit (issue #433 §4.5):
//!   - `prs_per_dev`, `pr_cycle_time`, `build_success` are not
//!     ingested (Bitbucket PRs / CI builds). The paired CH migration
//!     drops the predecessor's 3 NULL-only branches from the view.
//!     The corresponding FE-visible `metric_key`s remain in the
//!     response shape because this `query_ref` hardcodes them to NULL
//!     columns in the wide-aggregate — the honest-NULL → `ComingSoon`
//!     contract from `20260423120000_bullet-views-honest-nulls.sql`.
//!
//! IC variant: there is no `IC_BULLET_CODE_QUALITY` metric in the
//! seed (`m20260422_000001_seed_metrics`) — only the Team-scope
//! `…04` exists. Nothing to update on the IC side.
//!
//! Walker compatibility: the query has exactly two leaf subqueries
//! that read from `insight.code_quality_bullet_rows GROUP BY person_id`
//! (one in `p`, one in `inner_c`). `inject_date_filter_into_subqueries`
//! in `handlers.rs` walks both and injects
//! `WHERE metric_date >= … AND <` before the `GROUP BY` in each leaf —
//! same behavior as the predecessor.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_CODE_QUALITY_ID: &str = "00000000000000000001000000000004";

/// Inner wide-aggregate block: one row per `person_id` with every
/// FE-visible `metric_key` materialized in its own column.
///   - `bugs_fixed_v`: `sumIf` period sum (the one real metric).
///   - `prs_per_dev_v` / `pr_cycle_time_v` / `build_success_v`:
///     hardcoded NULL — the view no longer emits these (no ingestion
///     source). The honest-NULL contract from
///     `20260423120000_bullet-views-honest-nulls.sql` renders them as
///     `ComingSoon` on the FE.
///
/// `pp` is the output alias used by the caller.
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
                any(c.company_max) AS range_max \
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
                    max(v_period) AS company_max \
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

    /// Explicitly irreversible. The paired CH migration
    /// `20260520000000_code-quality-bullet-rewrite.sql` redefines
    /// `insight.code_quality_bullet_rows` to drop 3 `ComingSoon`
    /// NULL-only emissions. Restoring the old `query_ref` here without
    /// reverting the view would have the queries reach into a view
    /// that no longer emits those branches — the `multiIf` would fall
    /// through to `avg(metric_value)` over 0 rows = NULL (same
    /// observable result as `ComingSoon`, technically), but the
    /// roundtrip is misleading. Roll back by reverting the paired CH
    /// migration first, then this `down()`. Same pattern as
    /// `m20260518_000001_collab_bullet_rewrite` and
    /// `m20260519_000001_ai_bullet_rewrite`.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260520_000001_code_quality_bullet_rewrite is irreversible: \
             roll back the paired CH migration \
             20260520000000_code-quality-bullet-rewrite.sql (which drops \
             3 ComingSoon NULL-only emissions from the view) before \
             reverting metrics.query_ref."
                .to_string(),
        ))
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
    const EXPECTED_METRIC_KEYS: &[&str] =
        &["bugs_fixed", "prs_per_dev", "pr_cycle_time", "build_success"];

    /// The single raw `metric_key` the view emits that `query_ref`
    /// reads via `sumIf`. A typo here = silent NULL.
    const EXPECTED_RAW_KEYS_READ_BY_QUERY: &[&str] = &["bugs_fixed"];

    /// `metric_key`s that must NOT be read via `sumIf` — the view no
    /// longer emits them, so a `metric_key = 'X'` read would silently
    /// aggregate to NULL.
    const FORBIDDEN_RAW_KEY_READS: &[&str] =
        &["prs_per_dev", "pr_cycle_time", "build_success"];

    fn assert_query_shape(query: &str, label: &str) {
        let table_refs = query.matches("insight.code_quality_bullet_rows").count();
        assert_eq!(
            table_refs, 2,
            "{label}: expected 2 references to `insight.code_quality_bullet_rows`, got {table_refs}"
        );

        let person_groupbys = query.matches("GROUP BY person_id").count();
        assert_eq!(
            person_groupbys, 2,
            "{label}: expected 2 occurrences of `GROUP BY person_id`, got {person_groupbys}"
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
        // Team-scope: company-wide median (no IC variant exists for
        // this section, so no team_* labels should appear).
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
    }
}
