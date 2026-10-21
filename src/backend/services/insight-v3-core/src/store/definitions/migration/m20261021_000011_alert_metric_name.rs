//! A notification carries what its metric was called when it was owed, so a
//! message names the metric as people read it rather than by its id.
//!
//! Every row already stored keeps no name: its metric id is the name people
//! know it by.
//!
//! INVARIANT: the step is safe to run again. The column is added with
//! `IF NOT EXISTS` and nothing is written into it.

use sea_orm::DbErr;
use sea_orm_migration::prelude::*;

/// The same collation as the table, so a name compares the way the rest of
/// the row does.
const ADD_METRIC_NAME: &str = "ALTER TABLE alert_notifications ADD COLUMN IF NOT EXISTS metric_name VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL AFTER metric";

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261021_000011_alert_metric_name"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(ADD_METRIC_NAME)
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "dropping the metric names would lose what every notification called its metric"
                .to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::ADD_METRIC_NAME;

    #[test]
    fn the_name_is_added_once_and_left_empty_on_existing_rows() {
        assert!(
            ADD_METRIC_NAME.contains("ADD COLUMN IF NOT EXISTS metric_name"),
            "should be guarded: {ADD_METRIC_NAME}"
        );
        assert!(
            ADD_METRIC_NAME.ends_with("NULL AFTER metric"),
            "should be nullable beside the id: {ADD_METRIC_NAME}"
        );
        assert!(
            !ADD_METRIC_NAME.contains("DEFAULT"),
            "existing rows stay without a name: {ADD_METRIC_NAME}"
        );
    }
}
