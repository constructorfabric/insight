//! Fold the department peer-median into the IC KPIs `query_ref` (`…0010`),
//! so each KPI tile reads its benchmark off the same row that produces its
//! value — same cohort (department), same per-person rollup. This makes the
//! standalone `ic_kpi_peer_median` view (`…0037`) redundant.
//!
//! Shape: the existing per-person rollup becomes the `k` subquery (now also
//! carrying `org_unit_id`). A second use of the identical rollup is grouped
//! by `org_unit_id` into department medians (`<kpi>_median`) + cohort size
//! (`peer_n`), LEFT-joined back on `org_unit_id`. The cohort medians reuse
//! the EXACT per-person sum/avg rules the value uses, so value and median
//! are same-method by construction.
//!
//! Person scoping unchanged: the outer exposes `person_id` (from `k`), so
//! the handler's ` AND person_id = ?` selects the viewed person's row +
//! their department's medians. No outer GROUP BY (k is already one row per
//! person; d is one row per department; the join is 1:1 per person).
//!
//! Walker compatibility: `inject_date_filter_into_subqueries` recurses into
//! both `(… FROM insight.ic_kpis … GROUP BY person_id)` leaves (the `k`
//! rollup and the `per_person` rollup feeding the department medians) and
//! injects ` WHERE metric_date >= … AND metric_date <= …` before each
//! `GROUP BY person_id`, so both the value and the cohort are bounded to the
//! selected period.
//!
//! This migration also drops two now-redundant standalone metrics, made
//! obsolete by the peer-cohort consolidation:
//!   - `…0037` `IC KPI Peer Median` — its medians are folded into `…0010` above;
//!   - `…0034` `Peer Cohort Stats` — its per-bullet distribution is now carried
//!     on each bullet row (`m20260604_00000{1,2,4}` + `m20260606_000001`).
//!
//! Both pair with ingestion drops of the backing views. `down()` restores the
//! verbatim rows seeded by `m20260527_000002_seed_metric_views`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const IC_KPIS_ID: &str = "00000000000000000001000000000010";

const ZERO_TENANT: &str = "00000000000000000000000000000000";

/// `Peer Cohort Stats` (`…0034`) — dropped here; restored verbatim by `down()`
/// from `m20260527_000002_seed_metric_views`.
const PEER_COHORT_STATS_ID: &str = "00000000000000000001000000000034";
const PEER_COHORT_STATS_NAME: &str = "Peer Cohort Stats";
const PEER_COHORT_STATS_DESC: &str =
    "Aggregate percentiles per metric across a cohort (kind=ic|team). No per-person rows.";
const PEER_COHORT_STATS_QR: &str = "SELECT cohort_seed, kind, metric_key, quantileExact(0.25)(p25) AS p25, quantileExact(0.5)(p50) AS p50, quantileExact(0.75)(p75) AS p75, min(min) AS min, max(max) AS max, max(n) AS n FROM insight.peer_cohort_stats GROUP BY cohort_seed, kind, metric_key";

/// `IC KPI Peer Median` (`…0037`) — dropped here; restored verbatim by `down()`
/// from `m20260527_000002_seed_metric_views`.
const IC_KPI_PEER_MEDIAN_ID: &str = "00000000000000000001000000000037";
const IC_KPI_PEER_MEDIAN_NAME: &str = "IC KPI Peer Median";
const IC_KPI_PEER_MEDIAN_DESC: &str =
    "Per-supervisor cohort percentiles (p25/p50/p75/n) for each IC KPI key.";
const IC_KPI_PEER_MEDIAN_QR: &str = "SELECT cohort_seed, kpi_key, quantileExact(0.25)(person_total) AS p25, quantileExact(0.5)(person_total) AS p50, quantileExact(0.75)(person_total) AS p75, uniqExact(person_id) AS n FROM (SELECT supervisor_email AS cohort_seed, person_id, kpi_key, multiIf(kpi_key IN ('bugs_fixed','tasks_closed','prs_merged','ai_sessions'), sum(value), avg(value)) AS person_total FROM insight.ic_kpi_peer_median GROUP BY supervisor_email, person_id, kpi_key) GROUP BY cohort_seed, kpi_key";

/// Per-person rollup over `insight.ic_kpis` (daily rows → one row per
/// person for the period). Reused verbatim for both the value row (`k`)
/// and the department-median input (`per_person`) so the median is computed
/// from the same per-person aggregates as the value. Now also surfaces
/// `org_unit_id` (the department cohort key).
fn per_person_rollup() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         sum(loc) AS loc, \
         round(avg(ai_loc_share_pct), 1) AS ai_loc_share_pct, \
         sum(prs_merged) AS prs_merged, \
         avg(pr_cycle_time_h) AS pr_cycle_time_h, \
         round(avg(focus_time_pct), 1) AS focus_time_pct, \
         sum(tasks_closed) AS tasks_closed, \
         sum(bugs_fixed) AS bugs_fixed, \
         anyOrNull(build_success_pct) AS build_success_pct, \
         sum(ai_sessions) AS ai_sessions \
     FROM insight.ic_kpis \
     GROUP BY person_id"
}

fn new_query() -> String {
    let pp = per_person_rollup();
    format!(
        "SELECT k.person_id AS person_id, \
                k.loc AS loc, \
                k.ai_loc_share_pct AS ai_loc_share_pct, \
                k.prs_merged AS prs_merged, \
                k.pr_cycle_time_h AS pr_cycle_time_h, \
                k.focus_time_pct AS focus_time_pct, \
                k.tasks_closed AS tasks_closed, \
                k.bugs_fixed AS bugs_fixed, \
                k.build_success_pct AS build_success_pct, \
                k.ai_sessions AS ai_sessions, \
                d.loc_median AS loc_median, \
                d.ai_loc_share_pct_median AS ai_loc_share_pct_median, \
                d.prs_merged_median AS prs_merged_median, \
                d.pr_cycle_time_h_median AS pr_cycle_time_h_median, \
                d.focus_time_pct_median AS focus_time_pct_median, \
                d.tasks_closed_median AS tasks_closed_median, \
                d.bugs_fixed_median AS bugs_fixed_median, \
                d.build_success_pct_median AS build_success_pct_median, \
                d.ai_sessions_median AS ai_sessions_median, \
                d.peer_n AS peer_n \
         FROM ({pp}) k \
         LEFT JOIN ( \
             SELECT org_unit_id, \
                    quantileExact(0.5)(loc) AS loc_median, \
                    quantileExact(0.5)(ai_loc_share_pct) AS ai_loc_share_pct_median, \
                    quantileExact(0.5)(prs_merged) AS prs_merged_median, \
                    quantileExact(0.5)(pr_cycle_time_h) AS pr_cycle_time_h_median, \
                    quantileExact(0.5)(focus_time_pct) AS focus_time_pct_median, \
                    quantileExact(0.5)(tasks_closed) AS tasks_closed_median, \
                    quantileExact(0.5)(bugs_fixed) AS bugs_fixed_median, \
                    quantileExact(0.5)(build_success_pct) AS build_success_pct_median, \
                    quantileExact(0.5)(ai_sessions) AS ai_sessions_median, \
                    uniqExact(person_id) AS peer_n \
             FROM ({pp}) per_person \
             GROUP BY org_unit_id \
         ) d ON d.org_unit_id = k.org_unit_id"
    )
}

/// Predecessor `query_ref` as set by `m20260422_000001_seed_metrics.rs`
/// (the only migration that has touched `…0010`) — restored by `down()`.
fn old_query() -> &'static str {
    "SELECT person_id, sum(loc) AS loc, round(avg(ai_loc_share_pct), 1) AS ai_loc_share_pct, sum(prs_merged) AS prs_merged, avg(pr_cycle_time_h) AS pr_cycle_time_h, round(avg(focus_time_pct), 1) AS focus_time_pct, sum(tasks_closed) AS tasks_closed, sum(bugs_fixed) AS bugs_fixed, anyOrNull(build_success_pct) AS build_success_pct, sum(ai_sessions) AS ai_sessions FROM insight.ic_kpis GROUP BY person_id"
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{IC_KPIS_ID}')",
            qr = new_query().replace('\'', "''"),
        ))
        .await?;
        // Drop the now-redundant standalone peer-cohort metrics.
        for id in [PEER_COHORT_STATS_ID, IC_KPI_PEER_MEDIAN_ID] {
            db.execute_unprepared(&format!("DELETE FROM metrics WHERE id = UNHEX('{id}')"))
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{IC_KPIS_ID}')",
            qr = old_query().replace('\'', "''"),
        ))
        .await?;
        // Restore the verbatim rows the drops removed (seeded by
        // m20260527_000002_seed_metric_views).
        for (id, name, description, qr) in [
            (
                PEER_COHORT_STATS_ID,
                PEER_COHORT_STATS_NAME,
                PEER_COHORT_STATS_DESC,
                PEER_COHORT_STATS_QR,
            ),
            (
                IC_KPI_PEER_MEDIAN_ID,
                IC_KPI_PEER_MEDIAN_NAME,
                IC_KPI_PEER_MEDIAN_DESC,
                IC_KPI_PEER_MEDIAN_QR,
            ),
        ] {
            db.execute_unprepared(&format!(
                "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
                 VALUES (UNHEX('{id}'), UNHEX('{ZERO_TENANT}'), '{name}', '{description}', '{qr}', 1) \
                 ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref), is_enabled=VALUES(is_enabled)",
                qr = qr.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every KPI key whose value the FE renders must also carry a
    /// `<key>_median` column (department peer median) for the tile's
    /// vs-peer coloring + footer.
    const KPI_KEYS: &[&str] = &[
        "loc",
        "ai_loc_share_pct",
        "prs_merged",
        "pr_cycle_time_h",
        "focus_time_pct",
        "tasks_closed",
        "bugs_fixed",
        "build_success_pct",
        "ai_sessions",
    ];

    #[test]
    fn new_query_exposes_value_and_department_median_per_kpi() {
        let q = new_query();
        for key in KPI_KEYS {
            assert!(
                q.contains(&format!("k.{key} AS {key}")),
                "missing value column for {key}"
            );
            assert!(
                q.contains(&format!("{key}_median")),
                "missing department median column for {key}"
            );
        }
        assert!(
            q.contains("uniqExact(person_id) AS peer_n"),
            "missing peer_n"
        );
        assert!(
            q.contains("d.org_unit_id = k.org_unit_id"),
            "department cohort must join on org_unit_id"
        );
        // The cohort medians must roll per person first (same rules as the
        // value) — two uses of the identical rollup reading ic_kpis.
        assert_eq!(
            q.matches("FROM insight.ic_kpis").count(),
            2,
            "expected the per-person rollup twice (value row + cohort input)"
        );
        // Both rollups group per person so the date-walker injects before
        // each `GROUP BY person_id`.
        assert_eq!(q.matches("GROUP BY person_id").count(), 2);
    }

    #[test]
    fn down_restores_predecessor() {
        assert!(old_query().contains("FROM insight.ic_kpis GROUP BY person_id"));
        assert!(
            !old_query().contains("_median"),
            "predecessor had no peer medians"
        );
    }

    #[test]
    fn folds_the_two_redundant_metric_drops() {
        // The two standalone peer-cohort metrics this migration retires.
        assert_eq!(PEER_COHORT_STATS_ID, "00000000000000000001000000000034");
        assert_eq!(IC_KPI_PEER_MEDIAN_ID, "00000000000000000001000000000037");
        // down() restores their verbatim backing queries.
        assert!(PEER_COHORT_STATS_QR.contains("FROM insight.peer_cohort_stats"));
        assert!(IC_KPI_PEER_MEDIAN_QR.contains("FROM insight.ic_kpi_peer_median"));
    }
}
