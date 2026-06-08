//! Delete the now-redundant `peer_cohort_stats` metric (`…0034`). The
//! per-bullet cohort distribution (p25/p50/p75/min/max/n) is now carried on
//! each bullet row by the `m20260604_00000{1..5}_*_bullet_distribution`
//! migrations, so the FE no longer queries this metric. Pairs with ingestion
//! `20260604000001_drop-peer-cohort-stats.sql` (drops the view).
//!
//! Apply only after the FE that reads `row.peer` has shipped (the metric
//! 404s once deleted). `down()` restores the verbatim `…0034` row seeded by
//! `m20260527_000002_seed_metric_views.rs`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ID: &str = "00000000000000000001000000000034";
const ZERO_TENANT: &str = "00000000000000000000000000000000";
const NAME: &str = "Peer Cohort Stats";
const DESCRIPTION: &str =
    "Aggregate percentiles per metric across a cohort (kind=ic|team). No per-person rows.";
const QUERY_REF: &str = "SELECT cohort_seed, kind, metric_key, quantileExact(0.25)(p25) AS p25, quantileExact(0.5)(p50) AS p50, quantileExact(0.75)(p75) AS p75, min(min) AS min, max(max) AS max, max(n) AS n FROM insight.peer_cohort_stats GROUP BY cohort_seed, kind, metric_key";

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
