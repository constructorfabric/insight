use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

pub const REQUIRED_SOURCE_CHECKS: &[&str] = &[
    "chk_metric_sources_source_key_shape",
    "chk_metric_sources_schema_error_biconditional",
    "chk_metric_sources_schema_error_enum",
];

pub const REQUIRED_SOURCE_MEASURE_CHECKS: &[&str] = &[
    "chk_metric_source_measures_measure_key_shape",
    "chk_metric_source_measures_schema_error_biconditional",
    "chk_metric_source_measures_schema_error_enum",
];

pub const REQUIRED_SOURCE_DIMENSION_CHECKS: &[&str] = &[
    "chk_metric_source_dimensions_dimension_key_shape",
    "chk_metric_source_dimensions_display_order_nonnegative",
];

pub const REQUIRED_DEFINITION_CHECKS: &[&str] = &[
    "chk_metric_definitions_metric_key_shape",
    "chk_metric_definitions_entity_type_shape",
    "chk_metric_definitions_peer_cohort_key_shape",
    "chk_metric_definitions_computation_fields",
    "chk_metric_definitions_schema_error_biconditional",
    "chk_metric_definitions_schema_error_enum",
];

pub const REQUIRED_DIMENSION_CHECKS: &[&str] =
    &["chk_metric_definition_dimensions_display_order_nonnegative"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        for statement in SCHEMA_STATEMENTS {
            conn.execute_unprepared(statement).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

const SCHEMA_STATEMENTS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS metric_sources (
        id BINARY(16) NOT NULL PRIMARY KEY,
        tenant_id BINARY(16) NULL,
        tenant_id_sentinel BINARY(16) GENERATED ALWAYS AS (COALESCE(tenant_id, 0x00000000000000000000000000000000)) STORED,
        source_key VARCHAR(128) NOT NULL,
        source_kind ENUM('managed_observation','custom_observation_sql') NOT NULL,
        source_ref VARCHAR(256) NOT NULL,
        origin ENUM('builtin','custom') NOT NULL,
        is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
        schema_status ENUM('ok','error','unchecked') NOT NULL DEFAULT 'unchecked',
        schema_checked_at DATETIME(3) NULL,
        schema_error_code VARCHAR(64) NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_metric_sources_tenant_key (tenant_id_sentinel, source_key),
        CONSTRAINT chk_metric_sources_source_key_shape CHECK (source_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_sources_schema_error_biconditional CHECK ((schema_status = 'error') = (schema_error_code IS NOT NULL)),
        CONSTRAINT chk_metric_sources_schema_error_enum CHECK (schema_error_code IS NULL OR schema_error_code IN ('table_not_found','column_not_found','dimension_not_covered','unknown'))
    )",
    "CREATE TABLE IF NOT EXISTS metric_source_measures (
        id BINARY(16) NOT NULL PRIMARY KEY,
        source_id BINARY(16) NOT NULL,
        measure_key VARCHAR(128) NOT NULL,
        is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
        schema_status ENUM('ok','error','unchecked') NOT NULL DEFAULT 'unchecked',
        schema_checked_at DATETIME(3) NULL,
        schema_error_code VARCHAR(64) NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_metric_source_measures_key (source_id, measure_key),
        CONSTRAINT fk_metric_source_measures_source FOREIGN KEY (source_id) REFERENCES metric_sources(id) ON DELETE CASCADE,
        CONSTRAINT chk_metric_source_measures_measure_key_shape CHECK (measure_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_source_measures_schema_error_biconditional CHECK ((schema_status = 'error') = (schema_error_code IS NOT NULL)),
        CONSTRAINT chk_metric_source_measures_schema_error_enum CHECK (schema_error_code IS NULL OR schema_error_code IN ('table_not_found','column_not_found','dimension_not_covered','unknown'))
    )",
    "CREATE TABLE IF NOT EXISTS metric_source_dimensions (
        id BINARY(16) NOT NULL PRIMARY KEY,
        source_id BINARY(16) NOT NULL,
        dimension_key VARCHAR(64) NOT NULL,
        display_order INT NOT NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_metric_source_dimensions_key (source_id, dimension_key),
        CONSTRAINT fk_metric_source_dimensions_source FOREIGN KEY (source_id) REFERENCES metric_sources(id) ON DELETE CASCADE,
        CONSTRAINT chk_metric_source_dimensions_dimension_key_shape CHECK (dimension_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_source_dimensions_display_order_nonnegative CHECK (display_order >= 0)
    )",
    "CREATE TABLE IF NOT EXISTS metric_definitions (
        id BINARY(16) NOT NULL PRIMARY KEY,
        tenant_id BINARY(16) NULL,
        tenant_id_sentinel BINARY(16) GENERATED ALWAYS AS (COALESCE(tenant_id, 0x00000000000000000000000000000000)) STORED,
        metric_key VARCHAR(128) NOT NULL,
        label VARCHAR(128) NOT NULL,
        description VARCHAR(2048) NULL,
        explanation VARCHAR(4096) NULL,
        unit VARCHAR(32) NULL,
        format ENUM('integer','decimal','currency','percent') NOT NULL,
        direction ENUM('higher_is_better','lower_is_better','neutral') NOT NULL,
        entity_type VARCHAR(64) NOT NULL,
        computation_type ENUM('sum','ratio') NOT NULL,
        scale DOUBLE NULL,
        peer_cohort_key VARCHAR(64) NULL,
        origin ENUM('builtin','custom') NOT NULL,
        is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
        schema_status ENUM('ok','error','unchecked') NOT NULL DEFAULT 'unchecked',
        schema_checked_at DATETIME(3) NULL,
        schema_error_code VARCHAR(64) NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_metric_definitions_tenant_key (tenant_id_sentinel, metric_key),
        KEY idx_metric_definitions_metric_key (metric_key),
        CONSTRAINT chk_metric_definitions_metric_key_shape CHECK (metric_key REGEXP BINARY '^[a-z][a-z0-9_]*[.][a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_definitions_entity_type_shape CHECK (entity_type REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_definitions_peer_cohort_key_shape CHECK (peer_cohort_key IS NULL OR peer_cohort_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_definitions_computation_fields CHECK (
            (computation_type = 'sum' AND scale IS NULL)
            OR (computation_type = 'ratio' AND scale IS NOT NULL)
        ),
        CONSTRAINT chk_metric_definitions_schema_error_biconditional CHECK ((schema_status = 'error') = (schema_error_code IS NOT NULL)),
        CONSTRAINT chk_metric_definitions_schema_error_enum CHECK (schema_error_code IS NULL OR schema_error_code IN ('table_not_found','column_not_found','dimension_not_covered','unknown'))
    )",
    "CREATE TABLE IF NOT EXISTS metric_definition_inputs (
        id BINARY(16) NOT NULL PRIMARY KEY,
        metric_definition_id BINARY(16) NOT NULL,
        input_role ENUM('value','numerator','denominator') NOT NULL,
        source_measure_id BINARY(16) NOT NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_metric_definition_inputs_role_measure (metric_definition_id, input_role, source_measure_id),
        CONSTRAINT fk_metric_definition_inputs_definition FOREIGN KEY (metric_definition_id) REFERENCES metric_definitions(id) ON DELETE CASCADE,
        CONSTRAINT fk_metric_definition_inputs_measure FOREIGN KEY (source_measure_id) REFERENCES metric_source_measures(id) ON DELETE RESTRICT
    )",
    "CREATE TABLE IF NOT EXISTS metric_definition_dimensions (
        id BINARY(16) NOT NULL PRIMARY KEY,
        metric_definition_id BINARY(16) NOT NULL,
        source_dimension_id BINARY(16) NOT NULL,
        display_order INT NOT NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_metric_definition_dimensions_dimension (metric_definition_id, source_dimension_id),
        CONSTRAINT fk_metric_definition_dimensions_definition FOREIGN KEY (metric_definition_id) REFERENCES metric_definitions(id) ON DELETE CASCADE,
        CONSTRAINT fk_metric_definition_dimensions_source_dimension FOREIGN KEY (source_dimension_id) REFERENCES metric_source_dimensions(id) ON DELETE RESTRICT,
        CONSTRAINT chk_metric_definition_dimensions_display_order_nonnegative CHECK (display_order >= 0)
    )",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::error_code::ALL_METRIC_SCHEMA_ERROR_CODES;

    #[test]
    fn schema_error_check_lists_match_error_code_enum() {
        let expected = ALL_METRIC_SCHEMA_ERROR_CODES
            .iter()
            .map(|code| format!("'{}'", code.as_db()))
            .collect::<Vec<_>>()
            .join(",");
        let expected_clause = format!("schema_error_code IN ({expected})");

        let mut check_count = 0;
        for statement in SCHEMA_STATEMENTS {
            if statement.contains("schema_error_code IN (") {
                assert!(
                    statement.contains(&expected_clause),
                    "CHECK list out of sync with MetricSchemaErrorCode in: {statement}"
                );
                check_count += 1;
            }
        }
        assert_eq!(check_count, 3);
    }

    #[test]
    fn every_required_check_appears_in_schema() {
        let all_sql = SCHEMA_STATEMENTS.join("\n");
        for check in REQUIRED_SOURCE_CHECKS
            .iter()
            .chain(REQUIRED_SOURCE_MEASURE_CHECKS)
            .chain(REQUIRED_SOURCE_DIMENSION_CHECKS)
            .chain(REQUIRED_DEFINITION_CHECKS)
            .chain(REQUIRED_DIMENSION_CHECKS)
        {
            assert!(all_sql.contains(check), "missing CHECK {check}");
        }
    }
}
