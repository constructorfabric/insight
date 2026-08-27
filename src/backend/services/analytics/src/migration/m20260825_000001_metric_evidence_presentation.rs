use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;

const ADD_COLUMN: &str = "ALTER TABLE metric_source_measures \
     ADD COLUMN evidence_presentation JSON NULL AFTER evidence_granularity";

// Every schema-error CHECK lists the whole `MetricSchemaErrorCode` enum, so
// widening the enum widens all of them together — an installation that already
// holds data keeps the same accepted set as a fresh one.
const ERROR_CODE_CHECKS: &[(&str, &str, &str)] = &[
    (
        "metric_sources",
        "chk_metric_sources_evidence_error_enum",
        "evidence_schema_error_code",
    ),
    (
        "metric_sources",
        "chk_metric_sources_schema_error_enum",
        "schema_error_code",
    ),
    (
        "metric_source_measures",
        "chk_metric_source_measures_schema_error_enum",
        "schema_error_code",
    ),
    (
        "metric_definitions",
        "chk_metric_definitions_schema_error_enum",
        "schema_error_code",
    ),
];

// Replaces the granularity-only trigger: a presentation edit changes what the
// evidence rows are claimed to carry, which is exactly what the next sweep has
// to re-probe.
const MEASURE_UPDATE_TRIGGER: &str = "CREATE TRIGGER trg_metric_source_measures_evidence_update
     AFTER UPDATE ON metric_source_measures
     FOR EACH ROW
     BEGIN
         IF NOT (OLD.evidence_granularity <=> NEW.evidence_granularity)
             OR NOT (OLD.evidence_presentation <=> NEW.evidence_presentation) THEN
             UPDATE metric_sources
             SET evidence_schema_status = 'unchecked',
                 evidence_schema_checked_at = NULL,
                 evidence_schema_error_code = NULL
             WHERE id = NEW.source_id;
         END IF;
     END";

const ACCEPTED_ERROR_CODES: &str =
    "'table_not_found','column_not_found','detail_key_not_found','dimension_not_covered','unknown'";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        if !manager
            .has_column("metric_source_measures", "evidence_presentation")
            .await?
        {
            conn.execute_unprepared(ADD_COLUMN).await?;
        }

        for (table, constraint, column) in ERROR_CODE_CHECKS {
            conn.execute_unprepared(&format!(
                "ALTER TABLE {table} DROP CONSTRAINT IF EXISTS {constraint}"
            ))
            .await?;
            conn.execute_unprepared(&format!(
                "ALTER TABLE {table} ADD CONSTRAINT {constraint} \
                 CHECK ({column} IS NULL OR {column} IN ({ACCEPTED_ERROR_CODES}))"
            ))
            .await?;
        }

        conn.execute_unprepared(
            "DROP TRIGGER IF EXISTS trg_metric_source_measures_evidence_update",
        )
        .await?;
        conn.execute_unprepared(MEASURE_UPDATE_TRIGGER).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_measure_row_keeps_serving_without_a_declaration() {
        assert!(ADD_COLUMN.contains("JSON NULL"));
        assert!(!ADD_COLUMN.contains("NOT NULL"));
    }

    #[test]
    fn every_schema_error_check_accepts_the_whole_error_code_enum() {
        use crate::domain::metric_definitions::error_code::ALL_METRIC_SCHEMA_ERROR_CODES;

        let expected = ALL_METRIC_SCHEMA_ERROR_CODES
            .iter()
            .map(|code| format!("'{}'", code.as_db()))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(ACCEPTED_ERROR_CODES, expected);
        assert!(
            ERROR_CODE_CHECKS
                .iter()
                .any(|(_, _, column)| *column == "evidence_schema_error_code"),
            "the evidence status column is the one the detail-key probe writes"
        );
    }

    #[test]
    fn a_presentation_edit_unchecks_the_source() {
        assert!(
            MEASURE_UPDATE_TRIGGER
                .contains("OLD.evidence_presentation <=> NEW.evidence_presentation")
        );
        assert!(
            MEASURE_UPDATE_TRIGGER
                .contains("OLD.evidence_granularity <=> NEW.evidence_granularity")
        );
        assert!(MEASURE_UPDATE_TRIGGER.contains("evidence_schema_status = 'unchecked'"));
    }
}
