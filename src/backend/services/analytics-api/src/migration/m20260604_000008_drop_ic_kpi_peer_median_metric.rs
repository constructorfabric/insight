//! Delete the now-redundant `ic_kpi_peer_median` metric (`…0037`). The
//! department KPI medians are now folded into the IC KPIs `query_ref`
//! (`…0010`) by `m20260604_000006_ic_kpis_peer_median`, so the FE no longer
//! queries this metric. Pairs with ingestion
//! `20260604000002_drop-ic-kpi-peer-median.sql` (drops the view).
//!
//! Apply only after the FE that reads the KPI-row medians has shipped.
//! `down()` restores the verbatim `…0037` row seeded by
//! `m20260527_000002_seed_metric_views.rs`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ID: &str = "00000000000000000001000000000037";
const ZERO_TENANT: &str = "00000000000000000000000000000000";
const NAME: &str = "IC KPI Peer Median";
const DESCRIPTION: &str = "Per-supervisor cohort percentiles (p25/p50/p75/n) for each IC KPI key.";
const QUERY_REF: &str = "SELECT cohort_seed, kpi_key, quantileExact(0.25)(person_total) AS p25, quantileExact(0.5)(person_total) AS p50, quantileExact(0.75)(person_total) AS p75, uniqExact(person_id) AS n FROM (SELECT supervisor_email AS cohort_seed, person_id, kpi_key, multiIf(kpi_key IN ('bugs_fixed','tasks_closed','prs_merged','ai_sessions'), sum(value), avg(value)) AS person_total FROM insight.ic_kpi_peer_median GROUP BY supervisor_email, person_id, kpi_key) GROUP BY cohort_seed, kpi_key";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(&format!("DELETE FROM metrics WHERE id = UNHEX('{ID}')"))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(&format!(
                "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
                 VALUES (UNHEX('{ID}'), UNHEX('{ZERO_TENANT}'), '{NAME}', '{DESCRIPTION}', '{qr}', 1) \
                 ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref)",
                qr = QUERY_REF.replace('\'', "''"),
            ))
            .await?;
        Ok(())
    }
}
