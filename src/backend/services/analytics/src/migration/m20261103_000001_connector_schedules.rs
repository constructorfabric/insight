use sea_orm_migration::prelude::*;

pub(super) const REQUIRED_CHECKS: &[&str] = &[
    "chk_connector_schedules_revision",
    "chk_connector_schedules_crons",
];

pub(super) const REQUIRED_ACKNOWLEDGEMENT_CHECKS: &[&str] = &[
    "chk_schedule_ack_applied_crons",
    "chk_schedule_ack_deployment_crons",
    "chk_schedule_ack_applied_origin",
    "chk_schedule_ack_deployment_origin",
    "chk_schedule_ack_revisions",
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

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE OR REPLACE SQL SECURITY DEFINER VIEW connector_schedule_requests_v1 AS
             SELECT connector, source_id, revision, crons FROM connector_schedules",
            )
            .await?;

        manager.get_connection().execute_unprepared(
            "CREATE TABLE IF NOT EXISTS connector_schedule_acknowledgements (
                connector VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                source_id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                applied_revision BIGINT UNSIGNED NULL,
                applied_crons JSON NULL,
                applied_timezone VARCHAR(64) NULL,
                applied_origin VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NULL,
                suspended BOOLEAN NULL,
                deployment_crons JSON NOT NULL,
                deployment_origin VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                observed_at DATETIME(3) NULL,
                attempted_revision BIGINT UNSIGNED NOT NULL,
                attempted_at DATETIME(3) NOT NULL,
                last_error VARCHAR(512) NULL,
                PRIMARY KEY (connector, source_id),
                CONSTRAINT chk_schedule_ack_applied_crons CHECK (JSON_TYPE(applied_crons) = 'ARRAY'),
                CONSTRAINT chk_schedule_ack_deployment_crons CHECK (JSON_TYPE(deployment_crons) = 'ARRAY'),
                CONSTRAINT chk_schedule_ack_applied_origin CHECK (applied_origin IN ('api', 'annotation', 'descriptor', 'default')),
                CONSTRAINT chk_schedule_ack_deployment_origin CHECK (deployment_origin IN ('annotation', 'descriptor', 'default')),
                CONSTRAINT chk_schedule_ack_revisions CHECK (applied_revision IS NULL OR applied_revision <= attempted_revision)
            )"
        ).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}
