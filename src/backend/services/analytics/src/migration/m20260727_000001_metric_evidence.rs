use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_evidence_columns(manager).await?;
        replace_evidence_constraints(manager).await?;
        replace_evidence_triggers(manager).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

async fn add_evidence_columns(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let conn = manager.get_connection();
    let columns = [
        (
            "metric_sources",
            "evidence_ref",
            "ALTER TABLE metric_sources ADD COLUMN evidence_ref VARCHAR(256) NULL AFTER source_ref",
        ),
        (
            "metric_sources",
            "evidence_schema_status",
            "ALTER TABLE metric_sources ADD COLUMN evidence_schema_status ENUM('ok','error','unchecked') NOT NULL DEFAULT 'unchecked' AFTER schema_error_code",
        ),
        (
            "metric_sources",
            "evidence_schema_checked_at",
            "ALTER TABLE metric_sources ADD COLUMN evidence_schema_checked_at DATETIME(3) NULL AFTER evidence_schema_status",
        ),
        (
            "metric_sources",
            "evidence_schema_error_code",
            "ALTER TABLE metric_sources ADD COLUMN evidence_schema_error_code VARCHAR(64) NULL AFTER evidence_schema_checked_at",
        ),
        (
            "metric_source_measures",
            "evidence_granularity",
            "ALTER TABLE metric_source_measures ADD COLUMN evidence_granularity ENUM('event','source_summary','derived_population') NULL AFTER measure_key",
        ),
    ];
    for (table, column, ddl) in columns {
        if !manager.has_column(table, column).await? {
            conn.execute_unprepared(ddl).await?;
        }
    }
    Ok(())
}

async fn replace_evidence_constraints(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let conn = manager.get_connection();
    let constraints = [
        (
            "chk_metric_sources_evidence_error_biconditional",
            "ALTER TABLE metric_sources ADD CONSTRAINT chk_metric_sources_evidence_error_biconditional CHECK ((evidence_schema_status = 'error') = (evidence_schema_error_code IS NOT NULL))",
        ),
        (
            "chk_metric_sources_evidence_error_enum",
            "ALTER TABLE metric_sources ADD CONSTRAINT chk_metric_sources_evidence_error_enum CHECK (evidence_schema_error_code IS NULL OR evidence_schema_error_code IN ('table_not_found','column_not_found','unknown'))",
        ),
    ];
    for (name, ddl) in constraints {
        conn.execute_unprepared(&format!(
            "ALTER TABLE metric_sources DROP CONSTRAINT IF EXISTS {name}"
        ))
        .await?;
        conn.execute_unprepared(ddl).await?;
    }
    Ok(())
}

async fn replace_evidence_triggers(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let conn = manager.get_connection();
    let triggers = [
        (
            "trg_metric_sources_evidence_ref_invalidate",
            "CREATE TRIGGER trg_metric_sources_evidence_ref_invalidate
             BEFORE UPDATE ON metric_sources
             FOR EACH ROW
             BEGIN
                 IF NOT (OLD.evidence_ref <=> NEW.evidence_ref)
                     OR NOT (OLD.source_ref <=> NEW.source_ref) THEN
                     SET NEW.evidence_schema_status = 'unchecked';
                     SET NEW.evidence_schema_checked_at = NULL;
                     SET NEW.evidence_schema_error_code = NULL;
                 END IF;
             END",
        ),
        (
            "trg_metric_source_measures_evidence_insert",
            "CREATE TRIGGER trg_metric_source_measures_evidence_insert
             AFTER INSERT ON metric_source_measures
             FOR EACH ROW
             UPDATE metric_sources
             SET evidence_schema_status = 'unchecked',
                 evidence_schema_checked_at = NULL,
                 evidence_schema_error_code = NULL
             WHERE id = NEW.source_id",
        ),
        (
            "trg_metric_source_measures_evidence_update",
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
        ),
    ];
    for (name, ddl) in triggers {
        conn.execute_unprepared(&format!("DROP TRIGGER IF EXISTS {name}"))
            .await?;
        conn.execute_unprepared(ddl).await?;
    }
    Ok(())
}
