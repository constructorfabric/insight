use sea_orm::DbErr;
use sea_orm_migration::prelude::*;

const ADD_METRIC_NAME: &str = "ALTER TABLE alert_notifications ADD COLUMN IF NOT EXISTS metric_name VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL AFTER metric";

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261021_000010_alert_metric_name"
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
