//! Adds the oldest observation a metric definition currently holds. The schema
//! validator records the oldest `metric_date` its sweep found across the
//! definition's input measures, and clears the column when a sweep reads the
//! relation and finds nothing — so NULL means "no observation is held now",
//! covering both never-seen and retained-no-longer, which the probe cannot
//! tell apart and neither can a reader.
//!
//! Paired with `last_observed_date`, it separates a day no reading exists for
//! from a day measured as empty. Unlike that column it is not monotonic: it
//! follows the relation forward as retention drops the oldest rows.

use sea_orm_migration::prelude::*;

// IF NOT EXISTS keeps this idempotent forward-repair, matching the other
// metric_definitions migrations.
const ADD_COLUMN: &str = "ALTER TABLE metric_definitions \
     ADD COLUMN IF NOT EXISTS first_observed_date DATE NULL AFTER schema_error_code";

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
                 DROP COLUMN IF EXISTS first_observed_date",
            )
            .await?;
        Ok(())
    }
}
