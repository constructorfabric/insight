//! Rewrite the Team / IC Bullet Task Delivery `query_ref`s to consume the
//! new `task_delivery_bullet_rows` shape (issue #433 §4.1, §4.2).
//!
//! Pairs with ingestion migration
//! `20260515000000_task-delivery-bullet-rewrite.sql`, which drops the daily
//! % from the view for 4 ratio metrics and emits their raw num/den
//! counters instead. The `query_ref`s now reconstruct the composites as
//! `100 * Σnum / Σden` over the period — the only mathematically correct
//! period aggregation when daily denominators differ (CLAUDE.md
//! "Aggregation correctness").
//!
//! Mathematical changes:
//!   - `due_date_compliance`      avg(daily %) → 100 * `Σon_time` / `Σwith_due`
//!   - `bugs_to_task_ratio`       avg(daily %) → 100 * Σbugs / Σtasks
//!   - `flow_efficiency`          median(per-issue %) → 100 * Σdev / Σlead
//!     (clamped to ≤100)
//!   - `worklog_logging_accuracy` symmetric fold(daily %) → 100 * Σworklog / `Σin_progress`
//!     (clamped to ≤100 — the predecessor's symmetric folding was bounded
//!     to `[0, 100]` and the FE gauge expects that range; retroactive
//!     worklog can push the raw Σ/Σ ratio above 100 which would break
//!     the chart scale)
//!
//! Preserved unchanged:
//!   - `tasks_completed`, `stale_in_progress`        — period sum
//!   - `task_dev_time`, `mean_time_to_resolution`,
//!     `pickup_time`                                 — per-person median
//!     across issues
//!   - `task_reopen_rate`                            — signed-events ratio
//!     with ≥5-closure gate
//!   - `estimation_accuracy`                         — symmetric folding
//!     around 100 (silver layer only exposes already-averaged scalars per
//!     person-day, so a true Σnum/Σden split isn't expressible from view
//!     output — separate workstream to widen the silver shape)
//!
//! Structural change:
//!   - Replaced `multiIf(metric_key=X, dispatch)` with wide-aggregate
//!     (`sumIf` / `quantileExactIf` / composite-ratio formulas per
//!     `metric_key`) + `ARRAY JOIN` unpivot back to long format. Mirrors
//!     the pattern used in `m20260430_000001_update_git_bullet.rs`.
//!
//! Walker compatibility: each query has exactly two leaf subqueries that
//! read from `insight.task_delivery_bullet_rows GROUP BY person_id`
//! (one in `p`, one in `inner_c`). `inject_date_filter_into_subqueries`
//! in `handlers.rs` walks both and injects `WHERE metric_date >= … AND <`
//! before the `GROUP BY` in each leaf — same behavior as before.
//!
//! Note on duplication: the wide-aggregate + ARRAY JOIN block is written
//! twice (once in `p`, once in `inner_c`). This is intentional for this
//! PR — eliminating the dup requires CTE-style hoisting which conflicts
//! with the current `handlers.rs` parser. Tracked as issue #433 §3.4 for
//! a follow-up PR.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_DELIVERY_ID: &str = "00000000000000000001000000000003";
const IC_BULLET_DELIVERY_ID: &str = "00000000000000000001000000000011";

/// Time-bound metrics whose period distribution has a long right tail.
/// Use P95 for `range_max` so a single year-old issue closed in-window
/// doesn't blow the gauge scale to 600d.
const P95_LIST: &str = "'mean_time_to_resolution', 'task_dev_time', 'pickup_time'";

/// Inner wide-aggregate block: per-person resolved metrics for one row
/// per `person_id`, with every FE-visible metric materialized in its own
/// column. Composite ratios are computed here as `100 * Σnum / Σden`
/// using `sumIf` over the raw `metric_key`s emitted by the new view shape.
///
/// `pp` is the output alias used by the caller.
fn wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         sumIf(metric_value, metric_key = 'tasks_completed') AS tasks_completed_v, \
         sumIf(metric_value, metric_key = 'stale_in_progress') AS stale_in_progress_v, \
         quantileExactIf(0.5)(metric_value, metric_key = 'task_dev_time' AND isNotNull(metric_value)) AS task_dev_time_v, \
         quantileExactIf(0.5)(metric_value, metric_key = 'mean_time_to_resolution' AND isNotNull(metric_value)) AS mttr_v, \
         quantileExactIf(0.5)(metric_value, metric_key = 'pickup_time' AND isNotNull(metric_value)) AS pickup_time_v, \
         if(sumIf(metric_value, metric_key = 'task_reopen_rate' AND metric_value > 0) >= 5, \
            round((-sumIf(metric_value, metric_key = 'task_reopen_rate' AND metric_value < 0) \
                   / sumIf(metric_value, metric_key = 'task_reopen_rate' AND metric_value > 0)) * 100, 1), \
            CAST(NULL AS Nullable(Float64))) AS task_reopen_rate_v, \
         if(sumIf(metric_value, metric_key = 'due_date_with_due') > 0, \
            round(toFloat64(100) * sumIf(metric_value, metric_key = 'due_date_on_time') \
                                 / sumIf(metric_value, metric_key = 'due_date_with_due'), 1), \
            CAST(NULL AS Nullable(Float64))) AS due_date_compliance_v, \
         if(sumIf(metric_value, metric_key = 'tasks_completed') > 0, \
            round(toFloat64(100) * sumIf(metric_value, metric_key = 'bugs_fixed') \
                                 / sumIf(metric_value, metric_key = 'tasks_completed'), 1), \
            CAST(NULL AS Nullable(Float64))) AS bugs_to_task_ratio_v, \
         if(sumIf(metric_value, metric_key = 'flow_efficiency_den') > 0, \
            least(toFloat64(100), \
                  round(toFloat64(100) * sumIf(metric_value, metric_key = 'flow_efficiency_num') \
                                       / sumIf(metric_value, metric_key = 'flow_efficiency_den'), 1)), \
            CAST(NULL AS Nullable(Float64))) AS flow_efficiency_v, \
         if(sumIf(metric_value, metric_key = 'in_progress_seconds') > 0, \
            least(toFloat64(100), \
                  round(toFloat64(100) * sumIf(metric_value, metric_key = 'worklog_seconds') \
                                       / sumIf(metric_value, metric_key = 'in_progress_seconds'), 1)), \
            CAST(NULL AS Nullable(Float64))) AS worklog_logging_accuracy_v, \
         if(countIf(metric_key = 'estimation_accuracy' AND metric_value > 0 AND metric_value <= 200) > 0, \
            greatest(toFloat64(0), \
                     toFloat64(100) - avgIf(abs(toFloat64(100) - metric_value), \
                                             metric_key = 'estimation_accuracy' AND metric_value > 0 AND metric_value <= 200)), \
            CAST(NULL AS Nullable(Float64))) AS estimation_accuracy_v \
     FROM insight.task_delivery_bullet_rows \
     GROUP BY person_id"
}

/// `ARRAY JOIN` unpivot: turns one wide `pp` row (with N metric columns)
/// into N long rows `(metric_key, v_period)`. Order of entries is the
/// `metric_key` set exposed by the bullet section; FE renders bullets in
/// whatever order it lays them out.
fn array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('tasks_completed',           tasks_completed_v), \
         ('stale_in_progress',         stale_in_progress_v), \
         ('task_dev_time',             task_dev_time_v), \
         ('mean_time_to_resolution',   mttr_v), \
         ('pickup_time',               pickup_time_v), \
         ('task_reopen_rate',          task_reopen_rate_v), \
         ('due_date_compliance',       due_date_compliance_v), \
         ('bugs_to_task_ratio',        bugs_to_task_ratio_v), \
         ('flow_efficiency',           flow_efficiency_v), \
         ('worklog_logging_accuracy',  worklog_logging_accuracy_v), \
         ('estimation_accuracy',       estimation_accuracy_v) \
     ] AS kv"
}

/// `range_max` aggregator: P95 for time-tail metrics, plain max otherwise.
fn range_max_expr() -> String {
    format!("if(metric_key IN ({P95_LIST}), quantileExact(0.95)(v_period), max(v_period))")
}

fn team_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    let rmax = range_max_expr();
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
                    {rmax} AS company_max \
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

fn ic_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    let rmax = range_max_expr();
    format!(
        "SELECT p.metric_key AS metric_key, \
                avg(p.v_period) AS value, \
                any(c.team_median) AS median, \
                any(c.team_min) AS range_min, \
                any(c.team_max) AS range_max \
         FROM ( \
             SELECT person_id, org_unit_id, \
                    kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({pp}) pp \
             {kv} \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, org_unit_id, \
                    quantileExact(0.5)(v_period) AS team_median, \
                    min(v_period) AS team_min, \
                    {rmax} AS team_max \
             FROM ( \
                 SELECT person_id, org_unit_id, \
                        kv.1 AS metric_key, kv.2 AS v_period \
                 FROM ({pp}) ppc \
                 {kv} \
             ) inner_c \
             GROUP BY metric_key, org_unit_id \
         ) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
         GROUP BY p.metric_key"
    )
}

/// Predecessor `query_ref`s as set by
/// `m20260429_000001_task_delivery_silver_rewrite.rs` — used in `down()`
/// so a rollback restores the previous shape rather than the original
/// (long-obsolete) `m20260422` seed.
const OLD_SUM_LIST: &str = "'tasks_completed', 'stale_in_progress'";
const OLD_FOLD_LIST: &str = "'estimation_accuracy', 'worklog_logging_accuracy'";
const OLD_MEDIAN_LIST: &str =
    "'mean_time_to_resolution', 'task_dev_time', 'pickup_time', 'flow_efficiency'";

fn old_inner_v_period() -> String {
    format!(
        "multiIf(\
metric_key = 'task_reopen_rate', \
    if(sumIf(metric_value, metric_value > 0) >= 5, \
       round((-sumIf(metric_value, metric_value < 0) / sumIf(metric_value, metric_value > 0)) * 100, 1), \
       NULL), \
metric_key IN ({OLD_SUM_LIST}), sum(metric_value), \
metric_key IN ({OLD_MEDIAN_LIST}), quantileExact(0.5)(metric_value), \
metric_key IN ({OLD_FOLD_LIST}), \
    if(countIf(metric_value > 0 AND metric_value <= 200) > 0, \
       greatest(toFloat64(0), toFloat64(100) - avgIf(abs(toFloat64(100) - metric_value), metric_value > 0 AND metric_value <= 200)), \
       NULL), \
avg(metric_value)\
)"
    )
}

fn old_team_query() -> String {
    let v = old_inner_v_period();
    let rmax = range_max_expr();
    format!(
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.company_median) AS median, any(c.company_min) AS range_min, any(c.company_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, {v} AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, quantileExact(0.5)(v_period) AS company_median, min(v_period) AS company_min, {rmax} AS company_max FROM (SELECT metric_key, person_id, {v} AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key) c ON c.metric_key = p.metric_key GROUP BY p.metric_key"
    )
}

fn old_ic_query() -> String {
    let v = old_inner_v_period();
    let rmax = range_max_expr();
    format!(
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, {v} AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExact(0.5)(v_period) AS team_median, min(v_period) AS team_min, {rmax} AS team_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, {v} AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key"
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, query) in [
            (TEAM_BULLET_DELIVERY_ID, team_query()),
            (IC_BULLET_DELIVERY_ID, ic_query()),
        ] {
            db.execute_unprepared(&format!(
                "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{hex_id}')",
                qr = query.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, query) in [
            (TEAM_BULLET_DELIVERY_ID, old_team_query()),
            (IC_BULLET_DELIVERY_ID, old_ic_query()),
        ] {
            db.execute_unprepared(&format!(
                "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{hex_id}')",
                qr = query.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests are intentionally string-contains rather than full SQL
    // equality. The goal is to catch the high-impact regressions that a
    // typo in this PR would propagate to PR 4-7 (each of which copies
    // this pattern for a different bullet section):
    //   - misspelled `metric_key` literal (e.g. `due_date_on_time` →
    //     `due_date_ontime`) would silently aggregate to NULL because
    //     no row in the view matches.
    //   - missing composite-ratio formula or a clamped/unclamped
    //     mismatch between caller's expectations and current logic.
    //   - `p` and `inner_c` of the JOIN going out of sync — both must
    //     reference the same wide-aggregate shape (`{pp}` helper).
    //
    // Read-the-binary tests are easier to maintain than ClickHouse
    // round-trip tests and don't need a running cluster.

    /// Every FE-visible `metric_key` the bullet section emits must appear
    /// as an `('X', X_v)` entry in the ARRAY JOIN unpivot.
    const EXPECTED_METRIC_KEYS: &[&str] = &[
        "tasks_completed",
        "stale_in_progress",
        "task_dev_time",
        "mean_time_to_resolution",
        "pickup_time",
        "task_reopen_rate",
        "due_date_compliance",
        "bugs_to_task_ratio",
        "flow_efficiency",
        "worklog_logging_accuracy",
        "estimation_accuracy",
    ];

    /// Every raw `metric_key` the view emits that the `query_ref` reads
    /// via `sumIf`/`avgIf`/`countIf`/`quantileExactIf` must appear as a
    /// literal in the wide-aggregate. A typo here silently aggregates
    /// the column to NULL because no view row matches.
    const EXPECTED_RAW_KEYS_READ_BY_QUERY: &[&str] = &[
        "tasks_completed",
        "stale_in_progress",
        "task_dev_time",
        "mean_time_to_resolution",
        "pickup_time",
        "task_reopen_rate",
        "due_date_on_time",
        "due_date_with_due",
        "bugs_fixed",
        "flow_efficiency_num",
        "flow_efficiency_den",
        "worklog_seconds",
        "in_progress_seconds",
        "estimation_accuracy",
    ];

    fn assert_query_shape(query: &str, label: &str) {
        // Both sides of the JOIN read from the same source table.
        let table_refs = query.matches("insight.task_delivery_bullet_rows").count();
        assert_eq!(
            table_refs, 2,
            "{label}: expected 2 references to `insight.task_delivery_bullet_rows` (one per JOIN side, no CTE hoist yet — see issue #433 §3.4), got {table_refs}"
        );

        // Each side has its own GROUP BY person_id wide-aggregate.
        let person_groupbys = query.matches("GROUP BY person_id").count();
        assert_eq!(
            person_groupbys, 2,
            "{label}: expected 2 occurrences of `GROUP BY person_id` (p and inner_c), got {person_groupbys}"
        );

        // FE-visible metric_keys are unpivoted via ARRAY JOIN.
        for key in EXPECTED_METRIC_KEYS {
            let literal = format!("'{key}'");
            assert!(
                query.contains(&literal),
                "{label}: missing FE-visible metric_key literal {literal} in ARRAY JOIN unpivot"
            );
        }

        // Raw metric_keys the wide-aggregate reads from the view must
        // match what the view emits. A typo here = silent NULL.
        for key in EXPECTED_RAW_KEYS_READ_BY_QUERY {
            let read = format!("metric_key = '{key}'");
            assert!(
                query.contains(&read),
                "{label}: missing read of raw metric_key {key} (`metric_key = '{key}'`) in wide-aggregate"
            );
        }

        // worklog_logging_accuracy must be clamped to ≤100 — predecessor
        // used symmetric folding bounded to [0, 100]; FE gauge expects
        // that range.
        assert!(
            query.contains("worklog_logging_accuracy_v"),
            "{label}: missing worklog_logging_accuracy_v formula"
        );
        // Find the worklog_logging_accuracy_v formula and check it
        // contains a `least` call (the clamp). This is heuristic but
        // catches the most likely regression.
        let Some(worklog_start) = query.find("worklog_logging_accuracy_v") else {
            panic!("{label}: worklog_logging_accuracy_v not found");
        };
        // Look backward up to 400 chars for the start of the formula
        // (well-bounded by surrounding `if(...)` wrapper).
        let formula_start = worklog_start.saturating_sub(400);
        let formula_window = &query[formula_start..worklog_start];
        assert!(
            formula_window.contains("least(toFloat64(100)"),
            "{label}: worklog_logging_accuracy_v must be clamped via least(toFloat64(100), …); got:\n{formula_window}"
        );

        // flow_efficiency also clamped.
        let Some(flow_start) = query.find("flow_efficiency_v") else {
            panic!("{label}: flow_efficiency_v not found");
        };
        let formula_start = flow_start.saturating_sub(400);
        let formula_window = &query[formula_start..flow_start];
        assert!(
            formula_window.contains("least(toFloat64(100)"),
            "{label}: flow_efficiency_v must be clamped via least(toFloat64(100), …)"
        );
    }

    #[test]
    fn team_query_shape() {
        let q = team_query();
        assert_query_shape(&q, "team_query");
        // Team-scope: company-wide median (not partitioned by org_unit_id).
        assert!(
            q.contains("company_median") && q.contains("company_min") && q.contains("company_max"),
            "team_query must expose company_* range, got:\n{q}"
        );
        assert!(
            !q.contains("team_median"),
            "team_query must NOT use team_median (that's the IC-side label)"
        );
        // Outer join key is metric_key only (no org_unit_id pairing).
        assert!(
            q.contains("ON c.metric_key = p.metric_key"),
            "team_query JOIN must be on metric_key alone"
        );
    }

    #[test]
    fn ic_query_shape() {
        let q = ic_query();
        assert_query_shape(&q, "ic_query");
        // IC-scope: team-wide median (partitioned by org_unit_id).
        assert!(
            q.contains("team_median") && q.contains("team_min") && q.contains("team_max"),
            "ic_query must expose team_* range, got:\n{q}"
        );
        assert!(
            !q.contains("company_median"),
            "ic_query must NOT use company_median (that's the Team-side label)"
        );
        // Outer join key includes org_unit_id.
        assert!(
            q.contains("c.org_unit_id = p.org_unit_id"),
            "ic_query JOIN must include org_unit_id"
        );
    }
}
