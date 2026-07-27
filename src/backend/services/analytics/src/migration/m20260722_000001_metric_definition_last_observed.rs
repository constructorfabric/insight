//! Adds the data-freshness marker to metric definitions. The schema
//! validator records the newest `metric_date` observed across a
//! definition's input measures on every sweep; NULL means no observation
//! has ever been seen. Freshness is orthogonal to `schema_status`, which
//! stays purely structural.

use sea_orm_migration::prelude::*;

// IF NOT EXISTS keeps this idempotent forward-repair, matching the other
// metric_definitions migrations.
const ADD_COLUMN: &str = "ALTER TABLE metric_definitions \
     ADD COLUMN IF NOT EXISTS last_observed_date DATE NULL AFTER schema_error_code";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(ADD_COLUMN)
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE metric_definitions \
                 DROP COLUMN IF EXISTS last_observed_date",
            )
            .await?;
        Ok(())
    }
}
