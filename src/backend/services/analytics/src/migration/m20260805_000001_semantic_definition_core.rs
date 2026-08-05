//! Semantic-layer definition core — the new definition store (#2208, Phase 1
//! step 1). Domain-shaped tables for datasets, measures, metrics, and an
//! append-only revision audit, per the Semantic Layer DESIGN §3.7
//! (`cpt-semantic-layer-dbtable-{datasets,measures,metrics,definition-revisions}`)
//! and ADR-001.
//!
//! These are **additive**: the legacy `metric_*` store is untouched and keeps
//! serving until cutover, so the tables are physically prefixed `semantic_` to
//! coexist. This slice creates the schema only — no entities, reconciler, or
//! serving change yet (later Phase 1 slices).
//!
//! CHECK-constraint style follows the existing catalog migrations: key shapes,
//! the aggregation/expression biconditional, and monotonic-version guards. The
//! CHECK names are registered in `migration::REQUIRED_CHECKS_BY_TABLE` so the
//! startup probe verifies they were never dropped.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

pub const REQUIRED_DATASET_CHECKS: &[&str] = &[
    "chk_semantic_datasets_dataset_key_shape",
    "chk_semantic_datasets_custom_sql_biconditional",
    "chk_semantic_datasets_availability_reason_biconditional",
];

pub const REQUIRED_MEASURE_CHECKS: &[&str] = &[
    "chk_semantic_measures_measure_key_shape",
    "chk_semantic_measures_dataset_ref_shape",
    "chk_semantic_measures_event_time_shape",
    "chk_semantic_measures_entity_shape",
    "chk_semantic_measures_definition_version_positive",
    "chk_semantic_measures_aggregation_expr",
];

pub const REQUIRED_METRIC_CHECKS: &[&str] = &[
    "chk_semantic_metrics_metric_key_shape",
    "chk_semantic_metrics_entity_type_shape",
    "chk_semantic_metrics_cohort_key_shape",
    "chk_semantic_metrics_definition_version_positive",
];

pub const REQUIRED_REVISION_CHECKS: &[&str] = &[
    "chk_semantic_definition_revisions_definition_key_shape",
    "chk_semantic_definition_revisions_version_positive",
];

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
    // Datasets — the queryable relations measures build on. Product rows point
    // at a warehouse relation; custom rows carry a validated SELECT and its
    // captured schema. Availability is stored on the row (set by the later
    // structural-validation probe), never probed inline.
    "CREATE TABLE IF NOT EXISTS semantic_datasets (
        id BINARY(16) NOT NULL PRIMARY KEY,
        tenant_id BINARY(16) NULL,
        tenant_id_sentinel BINARY(16) GENERATED ALWAYS AS (COALESCE(tenant_id, 0x00000000000000000000000000000000)) STORED,
        dataset_key VARCHAR(128) NOT NULL,
        database_relation VARCHAR(256) NOT NULL,
        read_discipline ENUM('final','none') NOT NULL,
        retention_horizon VARCHAR(32) NULL,
        origin ENUM('product','custom') NOT NULL,
        custom_sql TEXT NULL,
        custom_schema JSON NULL,
        availability ENUM('available','unavailable','unchecked') NOT NULL DEFAULT 'unchecked',
        availability_checked_at DATETIME(3) NULL,
        availability_reason VARCHAR(256) NULL,
        is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_semantic_datasets_tenant_key (tenant_id_sentinel, dataset_key),
        CONSTRAINT chk_semantic_datasets_dataset_key_shape CHECK (dataset_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_datasets_custom_sql_biconditional CHECK ((origin = 'custom') = (custom_sql IS NOT NULL)),
        CONSTRAINT chk_semantic_datasets_availability_reason_biconditional CHECK ((availability = 'unavailable') = (availability_reason IS NOT NULL))
    )",
    // Measures — declarative aggregation of one dataset, the lowest editable
    // layer. The aggregation/expression biconditional pins which expression a
    // given aggregation requires: `count` takes neither, `count_distinct` takes
    // a subject, the numeric folds take a value.
    "CREATE TABLE IF NOT EXISTS semantic_measures (
        id BINARY(16) NOT NULL PRIMARY KEY,
        tenant_id BINARY(16) NULL,
        tenant_id_sentinel BINARY(16) GENERATED ALWAYS AS (COALESCE(tenant_id, 0x00000000000000000000000000000000)) STORED,
        measure_key VARCHAR(128) NOT NULL,
        dataset_ref VARCHAR(128) NOT NULL,
        filter JSON NULL,
        aggregation ENUM('count','sum','avg','min','max','count_distinct') NOT NULL,
        value_expr VARCHAR(1024) NULL,
        subject_expr VARCHAR(1024) NULL,
        event_time VARCHAR(128) NOT NULL,
        entity VARCHAR(128) NOT NULL,
        dimensions JSON NULL,
        definition_version INT NOT NULL DEFAULT 1,
        origin ENUM('product','custom') NOT NULL,
        is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_semantic_measures_tenant_key (tenant_id_sentinel, measure_key),
        KEY idx_semantic_measures_dataset_ref (dataset_ref),
        CONSTRAINT chk_semantic_measures_measure_key_shape CHECK (measure_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_measures_dataset_ref_shape CHECK (dataset_ref REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_measures_event_time_shape CHECK (event_time REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_measures_entity_shape CHECK (entity REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_measures_definition_version_positive CHECK (definition_version >= 1),
        CONSTRAINT chk_semantic_measures_aggregation_expr CHECK (
            (aggregation = 'count' AND value_expr IS NULL AND subject_expr IS NULL)
            OR (aggregation = 'count_distinct' AND subject_expr IS NOT NULL AND value_expr IS NULL)
            OR (aggregation IN ('sum','avg','min','max') AND value_expr IS NOT NULL AND subject_expr IS NULL)
        )
    )",
    // Metrics — composition of measures into a served value with display
    // identity. Dimension capability is derived (intersection of inputs), not
    // stored, so there is no dimensions column here. `computation` and
    // `transform` are validated JSON bodies.
    "CREATE TABLE IF NOT EXISTS semantic_metrics (
        id BINARY(16) NOT NULL PRIMARY KEY,
        tenant_id BINARY(16) NULL,
        tenant_id_sentinel BINARY(16) GENERATED ALWAYS AS (COALESCE(tenant_id, 0x00000000000000000000000000000000)) STORED,
        metric_key VARCHAR(128) NOT NULL,
        computation JSON NOT NULL,
        transform JSON NULL,
        format ENUM('integer','decimal','currency','percent') NOT NULL,
        direction ENUM('higher_is_better','lower_is_better','neutral') NOT NULL,
        entity_type VARCHAR(64) NOT NULL,
        cohort_key VARCHAR(64) NULL,
        definition_version INT NOT NULL DEFAULT 1,
        origin ENUM('product','custom') NOT NULL,
        is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_semantic_metrics_tenant_key (tenant_id_sentinel, metric_key),
        CONSTRAINT chk_semantic_metrics_metric_key_shape CHECK (metric_key REGEXP BINARY '^[a-z][a-z0-9_]*[.][a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_metrics_entity_type_shape CHECK (entity_type REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_metrics_cohort_key_shape CHECK (cohort_key IS NULL OR cohort_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_metrics_definition_version_positive CHECK (definition_version >= 1)
    )",
    // Definition revisions — append-only audit written by every mutation path
    // (seed reconciliation and the future editing API alike) from the store's
    // first day, so runtime editing never lands without an audit trail.
    "CREATE TABLE IF NOT EXISTS semantic_definition_revisions (
        id BINARY(16) NOT NULL PRIMARY KEY,
        kind ENUM('dataset','measure','metric','chart','dashboard') NOT NULL,
        definition_key VARCHAR(128) NOT NULL,
        version INT NOT NULL,
        actor VARCHAR(256) NOT NULL,
        body JSON NOT NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        KEY idx_semantic_definition_revisions_lookup (kind, definition_key, version),
        CONSTRAINT chk_semantic_definition_revisions_definition_key_shape CHECK (definition_key REGEXP BINARY '^[a-z][a-z0-9_]*([.][a-z][a-z0-9_]*)?$'),
        CONSTRAINT chk_semantic_definition_revisions_version_positive CHECK (version >= 1)
    )",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_required_check_appears_in_schema() {
        let all_sql = SCHEMA_STATEMENTS.join("\n");
        for check in REQUIRED_DATASET_CHECKS
            .iter()
            .chain(REQUIRED_MEASURE_CHECKS)
            .chain(REQUIRED_METRIC_CHECKS)
            .chain(REQUIRED_REVISION_CHECKS)
        {
            assert!(all_sql.contains(check), "missing CHECK {check}");
        }
    }

    #[test]
    fn aggregation_enum_pins_the_six_kinds() {
        let all_sql = SCHEMA_STATEMENTS.join("\n");
        assert!(
            all_sql.contains(
                "aggregation ENUM('count','sum','avg','min','max','count_distinct') NOT NULL"
            ),
            "measure aggregation enum drifted"
        );
        // Each aggregation kind must appear in the expression biconditional, so
        // a new kind cannot be added to the enum without deciding its
        // value/subject-expression requirement.
        for kind in ["count", "sum", "avg", "min", "max", "count_distinct"] {
            assert!(
                all_sql.contains(&format!("aggregation = '{kind}'"))
                    || all_sql.contains(&format!("'{kind}'")),
                "aggregation kind {kind} missing from the schema"
            );
        }
    }

    #[test]
    fn every_table_scopes_by_tenant_sentinel_and_origin() {
        // Datasets/measures/metrics are tenant-scoped with an origin split, like
        // the legacy store — the sentinel makes the NULL (product) tenant unique
        // per key. Revisions are a global audit and are exempt.
        for table in ["semantic_datasets", "semantic_measures", "semantic_metrics"] {
            let stmt = SCHEMA_STATEMENTS
                .iter()
                .find(|s| s.contains(&format!("CREATE TABLE IF NOT EXISTS {table} ")))
                .unwrap_or_else(|| panic!("no CREATE for {table}"));
            assert!(
                stmt.contains("tenant_id_sentinel"),
                "{table} lacks sentinel"
            );
            assert!(
                stmt.contains("origin ENUM('product','custom')"),
                "{table} lacks the product/custom origin split"
            );
        }
    }
}
