//! Task Delivery silver-only rewrite — update `query_ref` for
//! `TEAM_BULLET_DELIVERY` (UUID …03) and `IC_BULLET_DELIVERY` (UUID …11)
//! to know about the expanded `metric_key` set emitted by the rewritten
//! `insight.task_delivery_bullet_rows` view.
//!
//! Preserved `metric_keys` (5):
//!   `tasks_completed`, `task_dev_time`, `task_reopen_rate`,
//!   `due_date_compliance`, `estimation_accuracy`
//!
//! New `metric_keys` (4):
//!   `worklog_logging_accuracy` — folded symmetric around 100 (raw daily
//!                                ratio = worklog ÷ time-in-dev-statuses).
//!   `bugs_to_task_ratio`       — default `avg(metric_value)`.
//!   `mean_time_to_resolution`  — period-level median (`quantileExact(0.5)`).
//!                                Robust against year-old issues finally
//!                                closed in-window dragging the average up.
//!   `stale_in_progress`        — summed across the period.
//!
//! `task_reopen_rate` rewritten as a period-aligned ratio of sums: bullet
//! rows are emitted with sign (+1 per close event, -1 per reopen event)
//! and the inner aggregate computes -sum(neg)/sum(pos) × 100. Suppressed
//! to NULL until ≥5 closures accumulate so a single 1-close/1-reopen
//! rebound doesn't read as 100%.
//!
//! Paired with CH migration `20260429000000_task-delivery-silver-
//! rewrite.sql` which redefines `insight.jira_closed_tasks` as a
//! silver-derived VIEW (was a `MergeTree` TABLE) and rewrites
//! `insight.task_delivery_bullet_rows` to emit 9 `metric_keys` instead
//! of 5. CH and Rust must apply together; until both land the new
//! bullets render as `ComingSoon`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_DELIVERY_ID: &str = "00000000000000000001000000000003";
const IC_BULLET_DELIVERY_ID: &str = "00000000000000000001000000000011";

const SUM_LIST: &str = "'tasks_completed', 'stale_in_progress'";
const FOLD_LIST: &str = "'estimation_accuracy', 'worklog_logging_accuracy'";
const MEDIAN_LIST: &str =
    "'mean_time_to_resolution', 'task_dev_time', 'pickup_time', 'flow_efficiency'";
/// Time-bound metrics whose distribution has a long right tail. Use P95
/// for the chart `range_max` instead of `max()` so a single year-old issue
/// closed in-window doesn't blow the gauge scale to 600d.
const P95_LIST: &str = "'mean_time_to_resolution', 'task_dev_time', 'pickup_time'";

/// `multiIf` body for the inner per-(`metric_key`, `person_id`) aggregate.
///
/// Special branches:
///  - `task_reopen_rate`: bullet rows are tagged with sign — +1 per close
///    event, -1 per reopen event — both scoped to the `OData` period.
///    Rate = -sum(neg)/sum(pos) × 100, NULL until at least 5 closures
///    accumulate (low-N denominators read as 100% and dominate).
///  - `SUM_LIST`: period total.
///  - `MEDIAN_LIST`: per-person median across the period — robust against
///    a single year-old issue closed in-window.
///  - `FOLD_LIST`: symmetric folding around 100 (raw daily ratios fold to
///    a 0..100 accuracy).
fn inner_v_period() -> String {
    format!(
        "multiIf(\
metric_key = 'task_reopen_rate', \
    if(sumIf(metric_value, metric_value > 0) >= 5, \
       round((-sumIf(metric_value, metric_value < 0) / sumIf(metric_value, metric_value > 0)) * 100, 1), \
       NULL), \
metric_key IN ({SUM_LIST}), sum(metric_value), \
metric_key IN ({MEDIAN_LIST}), quantileExact(0.5)(metric_value), \
metric_key IN ({FOLD_LIST}), \
    if(countIf(metric_value > 0 AND metric_value <= 200) > 0, \
       greatest(toFloat64(0), toFloat64(100) - avgIf(abs(toFloat64(100) - metric_value), metric_value > 0 AND metric_value <= 200)), \
       NULL), \
avg(metric_value)\
)"
    )
}

/// `range_max` aggregator: P95 for time/long-tail metrics, plain max
/// otherwise. Inlined into both queries.
fn range_max_expr() -> String {
    format!("if(metric_key IN ({P95_LIST}), quantileExact(0.95)(v_period), max(v_period))")
}

fn team_query() -> String {
    let v = inner_v_period();
    let rmax = range_max_expr();
    format!(
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.company_median) AS median, any(c.company_min) AS range_min, any(c.company_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, {v} AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, quantileExact(0.5)(v_period) AS company_median, min(v_period) AS company_min, {rmax} AS company_max FROM (SELECT metric_key, person_id, {v} AS v_period FROM insight.task_delivery_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key) c ON c.metric_key = p.metric_key GROUP BY p.metric_key"
    )
}

fn ic_query() -> String {
    let v = inner_v_period();
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
            let qr = query.replace('\'', "''");
            db.execute_unprepared(&format!(
                "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{hex_id}')"
            ))
            .await?;
        }

        Ok(())
    }

    /// Explicitly irreversible. The paired CH migration
    /// `20260429000000_task-delivery-silver-rewrite.sql` redefines
    /// `insight.task_delivery_bullet_rows` to emit a different
    /// `metric_key` set and replaces `insight.jira_closed_tasks` with a
    /// VIEW. Restoring `metrics.query_ref` here without first reverting
    /// the CH migration would leave queries pointing at `metric_keys` the
    /// view no longer emits — bullets would silently render
    /// `ComingSoon`. Roll back by re-running the previous CH migrations
    /// (`20260423120000_bullet-views-honest-nulls.sql` plus
    /// `20260422000000_gold-views.sql` to restore the `MergeTree` table)
    /// before reverting `metrics.query_ref`.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260429_000001_task_delivery_silver_rewrite is irreversible: \
             roll back the paired CH migration 20260429000000_task-delivery-silver-rewrite.sql \
             (re-run 20260423120000_bullet-views-honest-nulls.sql and \
             20260422000000_gold-views.sql) before reverting metrics.query_ref."
                .to_string(),
        ))
    }
}
