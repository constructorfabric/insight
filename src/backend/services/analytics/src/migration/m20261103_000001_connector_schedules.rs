use sea_orm_migration::prelude::*;

pub(super) const REQUIRED_CHECKS: &[&str] = &[
    "chk_connector_schedules_revision",
    "chk_connector_schedules_crons",
];

#[derive(Debug, DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS connector_schedules (
                connector VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                source_id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                crons JSON NOT NULL,
                revision BIGINT UNSIGNED NOT NULL,
                updated_by BINARY(16) NOT NULL,
                updated_at DATETIME(3) NOT NULL,
                PRIMARY KEY (connector, source_id),
                CONSTRAINT chk_connector_schedules_revision CHECK (revision >= 1),
                CONSTRAINT chk_connector_schedules_crons CHECK (JSON_TYPE(crons) = 'ARRAY')
            )",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}
