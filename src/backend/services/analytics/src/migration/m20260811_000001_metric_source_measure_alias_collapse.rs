//! Adds the alias-collapse rule to source measures. `sum` is the default, so
//! existing rows keep additive behavior.

use sea_orm_migration::prelude::*;

// IF NOT EXISTS: idempotent forward-repair, as the other catalog migrations.
const ADD_COLUMN: &str = "ALTER TABLE metric_source_measures \
     ADD COLUMN IF NOT EXISTS alias_collapse \
     ENUM('sum', 'max', 'min') NOT NULL DEFAULT 'sum' \
     AFTER evidence_granularity";

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
                "ALTER TABLE metric_source_measures \
                 DROP COLUMN IF EXISTS alias_collapse",
            )
            .await?;
        Ok(())
    }
}
