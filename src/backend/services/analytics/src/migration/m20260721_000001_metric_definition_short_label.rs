//! Adds the optional compact display label to metric definitions. Dense
//! surfaces (member grids, heatmap columns) render `short_label` when
//! present and fall back to `label`; NULL means the full label is already
//! compact enough.

use sea_orm_migration::prelude::*;

// IF NOT EXISTS keeps this idempotent forward-repair, matching the other
// metric_definitions migrations.
const ADD_COLUMN: &str = "ALTER TABLE metric_definitions \
     ADD COLUMN IF NOT EXISTS short_label VARCHAR(64) NULL AFTER label";

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
                 DROP COLUMN IF EXISTS short_label",
            )
            .await?;
        Ok(())
    }
}
