use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        conn.execute_unprepared(
            "ALTER TABLE metric_sources
             ADD COLUMN IF NOT EXISTS evidence_ref VARCHAR(256) NULL AFTER source_ref",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE metric_sources
             ADD COLUMN IF NOT EXISTS evidence_schema_status ENUM('ok','error','unchecked') NOT NULL DEFAULT 'unchecked' AFTER schema_error_code",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE metric_sources
             ADD COLUMN IF NOT EXISTS evidence_schema_checked_at DATETIME(3) NULL AFTER evidence_schema_status",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE metric_sources
             ADD COLUMN IF NOT EXISTS evidence_schema_error_code VARCHAR(64) NULL AFTER evidence_schema_checked_at",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE metric_sources
             DROP CONSTRAINT IF EXISTS chk_metric_sources_evidence_error_biconditional,
             ADD CONSTRAINT chk_metric_sources_evidence_error_biconditional CHECK ((evidence_schema_status = 'error') = (evidence_schema_error_code IS NOT NULL))",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE metric_sources
             DROP CONSTRAINT IF EXISTS chk_metric_sources_evidence_error_enum,
             ADD CONSTRAINT chk_metric_sources_evidence_error_enum CHECK (evidence_schema_error_code IS NULL OR evidence_schema_error_code IN ('table_not_found','column_not_found','unknown'))",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE metric_source_measures
             ADD COLUMN IF NOT EXISTS evidence_granularity ENUM('event','source_summary','derived_population') NULL AFTER measure_key",
        )
        .await?;
        conn.execute_unprepared(
            "DROP TRIGGER IF EXISTS trg_metric_sources_evidence_ref_invalidate",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE TRIGGER trg_metric_sources_evidence_ref_invalidate
             BEFORE UPDATE ON metric_sources
             FOR EACH ROW
             BEGIN
                 IF NOT (OLD.evidence_ref <=> NEW.evidence_ref) THEN
                     SET NEW.evidence_schema_status = 'unchecked';
                     SET NEW.evidence_schema_checked_at = NULL;
                     SET NEW.evidence_schema_error_code = NULL;
                 END IF;
             END",
        )
        .await?;
        conn.execute_unprepared(
            "DROP TRIGGER IF EXISTS trg_metric_source_measures_evidence_insert",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE TRIGGER trg_metric_source_measures_evidence_insert
             AFTER INSERT ON metric_source_measures
             FOR EACH ROW
             UPDATE metric_sources
             SET evidence_schema_status = 'unchecked',
                 evidence_schema_checked_at = NULL,
                 evidence_schema_error_code = NULL
             WHERE id = NEW.source_id",
        )
        .await?;
        conn.execute_unprepared(
            "DROP TRIGGER IF EXISTS trg_metric_source_measures_evidence_update",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE TRIGGER trg_metric_source_measures_evidence_update
             AFTER UPDATE ON metric_source_measures
             FOR EACH ROW
             BEGIN
                 IF NOT (OLD.evidence_granularity <=> NEW.evidence_granularity) THEN
                     UPDATE metric_sources
                     SET evidence_schema_status = 'unchecked',
                         evidence_schema_checked_at = NULL,
                         evidence_schema_error_code = NULL
                     WHERE id = NEW.source_id;
                 END IF;
             END",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}
