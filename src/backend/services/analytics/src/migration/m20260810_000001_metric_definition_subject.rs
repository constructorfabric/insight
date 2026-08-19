//! Adds the grouping subject to metric definitions. A subject is the single
//! topic a metric belongs to within its family (e.g. `meetings`, `messages`),
//! letting a surface that lists a whole family partition it into topics rather
//! than only sorting by name. Exactly one subject per metric — the partition
//! guarantee that a source key or a dimension cannot provide. NULL means the
//! metric declares no subject (custom metrics may omit it).

use sea_orm_migration::prelude::*;

// IF NOT EXISTS keeps this idempotent forward-repair, matching the other
// metric_definitions column migrations.
const ADD_COLUMN: &str = "ALTER TABLE metric_definitions \
     ADD COLUMN IF NOT EXISTS subject VARCHAR(64) NULL AFTER short_label";

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
                 DROP COLUMN IF EXISTS subject",
            )
            .await?;
        Ok(())
    }
}
