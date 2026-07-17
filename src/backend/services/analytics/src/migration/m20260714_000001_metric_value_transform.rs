//! Adds the post-aggregation value transform to metric definitions:
//! `y = clamp(clamp_min, clamp_max, multiplier * x + offset)`, applied by the
//! compiler to every computed value (period, peer pool entries, timeseries
//! points, breakdowns, histogram events).
//!
//! The transform keeps the computation vocabulary a closed algebra: bounded
//! index metrics (efficiency clamps, accuracy folds) express their final
//! shaping as data instead of growing bespoke computation variants. All four
//! columns NULL = identity (the repository loads that as no transform).

use sea_orm_migration::prelude::*;

// IF NOT EXISTS keeps this idempotent forward-repair, matching the other
// metric_definitions migrations.
const ADD_COLUMNS: &str = "ALTER TABLE metric_definitions \
     ADD COLUMN IF NOT EXISTS transform_multiplier DOUBLE NULL, \
     ADD COLUMN IF NOT EXISTS transform_offset DOUBLE NULL, \
     ADD COLUMN IF NOT EXISTS transform_clamp_min DOUBLE NULL, \
     ADD COLUMN IF NOT EXISTS transform_clamp_max DOUBLE NULL";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(ADD_COLUMNS)
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE metric_definitions \
                 DROP COLUMN IF EXISTS transform_multiplier, \
                 DROP COLUMN IF EXISTS transform_offset, \
                 DROP COLUMN IF EXISTS transform_clamp_min, \
                 DROP COLUMN IF EXISTS transform_clamp_max",
            )
            .await?;
        Ok(())
    }
}
