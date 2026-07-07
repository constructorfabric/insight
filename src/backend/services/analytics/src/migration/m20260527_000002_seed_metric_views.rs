//! Seed metric catalog rows for IC and team aggregate views.
//!
//! Each metric maps to a ClickHouse view created by the ingestion
//! migration `20260527000000_metrics-gold-views.sql`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Metric seed row: (`hex_id`, name, description, `query_ref`).
const SEEDS: &[(&str, &str, &str, &str)] = &[
    // ─── IC HISTOGRAM ──────────────────────────────────────────────
    (
        "00000000000000000001000000000030",
        "IC Histogram",
        "Distribution bins per IC metric per person per period. Filter by metric_key.",
        "SELECT person_id, metric_key, bin, bin_end, sum(count) AS count FROM insight.ic_histogram GROUP BY person_id, metric_key, bin, bin_end",
    ),
    // ─── PEER COHORT STATS ─────────────────────────────────────────
    (
        "00000000000000000001000000000034",
        "Peer Cohort Stats",
        "Aggregate percentiles per metric across a cohort (kind=ic|team). No per-person rows.",
        "SELECT cohort_seed, kind, metric_key, quantileExact(0.25)(p25) AS p25, quantileExact(0.5)(p50) AS p50, quantileExact(0.75)(p75) AS p75, min(min) AS min, max(max) AS max, max(n) AS n FROM insight.peer_cohort_stats GROUP BY cohort_seed, kind, metric_key",
    ),
    // ─── IC SECTION TREND ──────────────────────────────────────────
    (
        "00000000000000000001000000000036",
        "IC Section Trend",
        "Daily time series per (person, section, series_key). Long format.",
        "SELECT person_id, section_id, series_key, metric_date, sum(value) AS value FROM insight.ic_section_trend GROUP BY person_id, section_id, series_key, metric_date",
    ),
    // ─── IC KPI PEER MEDIAN ────────────────────────────────────────
    // Two-stage aggregation: per (supervisor, person, kpi) rollup using
    // sum (counters) or avg (ratios), then quantile across the cohort.
    (
        "00000000000000000001000000000037",
        "IC KPI Peer Median",
        "Per-supervisor cohort percentiles (p25/p50/p75/n) for each IC KPI key.",
        "SELECT cohort_seed, kpi_key, quantileExact(0.25)(person_total) AS p25, quantileExact(0.5)(person_total) AS p50, quantileExact(0.75)(person_total) AS p75, uniqExact(person_id) AS n FROM (SELECT supervisor_email AS cohort_seed, person_id, kpi_key, multiIf(kpi_key IN ('bugs_fixed','tasks_closed','prs_merged','ai_sessions'), sum(value), avg(value)) AS person_total FROM insight.ic_kpi_peer_median GROUP BY supervisor_email, person_id, kpi_key) GROUP BY cohort_seed, kpi_key",
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
