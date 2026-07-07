//! Point the `IC Chart LOC Trend` metric at the git LOC breakdown.
//!
//! The `insight.ic_chart_loc` view exposes per-person per-week git
//! file-category line counts (code, spec, config). This migration sets the
//! metric's `query_ref` to project those columns.
//!
//! UUID matches `insight-front/src/screensets/insight/api/metricRegistry.ts`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const LOC_METRIC_ID: &str = "00000000000000000001000000000014";
const LOC_METRIC_NAME: &str = "IC Chart LOC Trend";
const LOC_METRIC_DESCRIPTION: &str = "Weekly LOC breakdown: code, spec, config lines";
const LOC_METRIC_QUERY: &str = "SELECT date_bucket, code_loc, spec_lines, config_loc, person_id, metric_date FROM insight.ic_chart_loc";

const ZERO_TENANT: &str = "00000000000000000000000000000000";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(&format!(
            "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
             VALUES (UNHEX('{LOC_METRIC_ID}'), UNHEX('{ZERO_TENANT}'), '{LOC_METRIC_NAME}', '{LOC_METRIC_DESCRIPTION}', '{qr}', 1) \
             ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref)",
            qr = LOC_METRIC_QUERY.replace('\'', "''"),
        ))
        .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260624_000003_ic_chart_loc_git_breakdown is irreversible: \
             it overwrites the IC Chart LOC Trend query_ref in place. Restore \
             the prior value manually if needed."
                .to_string(),
        ))
    }
}
