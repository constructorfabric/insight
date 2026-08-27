use sea_orm_migration::prelude::*;

const ADD_COLUMN: &str = "ALTER TABLE metric_definitions \
     ADD COLUMN IF NOT EXISTS denominator_aggregation \
     ENUM('sum', 'distinct_count') NOT NULL DEFAULT 'sum' AFTER scale";

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
                 DROP COLUMN IF EXISTS denominator_aggregation",
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_ratios_keep_sum_denominators() {
        assert!(ADD_COLUMN.contains("NOT NULL DEFAULT 'sum'"));
        assert!(ADD_COLUMN.contains("'distinct_count'"));
    }
}
