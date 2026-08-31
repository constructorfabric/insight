//! Caching policy and coverage for the semantic layer's measures.
//!
//! Policy is not semantics: it says whether and how often a measure's work is
//! materialized, never what the measure means, so nothing here may move a
//! `definition_version`. Coverage is written by the refresher alone, after a
//! build lands, and states the dates the cache can be answered from.

use sea_orm_migration::prelude::*;

pub const REQUIRED_POLICY_CHECKS: &[&str] = &[
    "chk_semantic_cache_policies_measure_key_shape",
    "chk_semantic_cache_policies_hot_window_positive",
    "chk_semantic_cache_policies_refresh_interval_positive",
];

pub const REQUIRED_COVERAGE_CHECKS: &[&str] = &[
    "chk_semantic_cache_coverage_measure_key_shape",
    "chk_semantic_cache_coverage_definition_version_positive",
    "chk_semantic_cache_coverage_window_ordered",
];

#[derive(DeriveMigrationName)]
pub struct Migration;

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
    "CREATE TABLE IF NOT EXISTS semantic_cache_policies (
        id BINARY(16) NOT NULL PRIMARY KEY,
        measure_key VARCHAR(128) NOT NULL,
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        hot_window_days INT NOT NULL DEFAULT 35,
        refresh_interval_minutes INT NOT NULL DEFAULT 60,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_semantic_cache_policies_measure (measure_key),
        CONSTRAINT chk_semantic_cache_policies_measure_key_shape CHECK (measure_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_cache_policies_hot_window_positive CHECK (hot_window_days >= 1),
        CONSTRAINT chk_semantic_cache_policies_refresh_interval_positive CHECK (refresh_interval_minutes >= 1)
    )",
    // One row per measure and version: a version change starts its own coverage
    // rather than inheriting the dates the previous definition's rows covered.
    "CREATE TABLE IF NOT EXISTS semantic_cache_coverage (
        id BINARY(16) NOT NULL PRIMARY KEY,
        measure_key VARCHAR(128) NOT NULL,
        definition_version INT NOT NULL,
        covered_from DATE NOT NULL,
        covered_to DATE NOT NULL,
        refreshed_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_semantic_cache_coverage_measure_version (measure_key, definition_version),
        CONSTRAINT chk_semantic_cache_coverage_measure_key_shape CHECK (measure_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_semantic_cache_coverage_definition_version_positive CHECK (definition_version >= 1),
        CONSTRAINT chk_semantic_cache_coverage_window_ordered CHECK (covered_to >= covered_from)
    )",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_required_check_appears_in_schema() {
        let all_sql = SCHEMA_STATEMENTS.join("\n");
        for check in REQUIRED_POLICY_CHECKS
            .iter()
            .chain(REQUIRED_COVERAGE_CHECKS)
        {
            assert!(all_sql.contains(check), "missing CHECK {check}");
        }
    }

    #[test]
    fn a_measure_is_cached_under_one_policy_and_one_coverage_row_per_version() {
        let all_sql = SCHEMA_STATEMENTS.join("\n");
        assert!(all_sql.contains("UNIQUE KEY uq_semantic_cache_policies_measure (measure_key)"));
        assert!(all_sql.contains(
            "UNIQUE KEY uq_semantic_cache_coverage_measure_version (measure_key, definition_version)"
        ));
    }

    #[test]
    fn policy_carries_no_column_that_could_state_what_a_measure_means() {
        let policy = SCHEMA_STATEMENTS
            .iter()
            .find(|statement| statement.contains("semantic_cache_policies"))
            .unwrap_or_else(|| panic!("no CREATE for semantic_cache_policies"));

        for semantic in [
            "definition_version",
            "aggregation",
            "filter",
            "value_expr",
            "subject_expr",
            "dimensions",
        ] {
            assert!(
                !policy.contains(semantic),
                "policy must not carry `{semantic}` — caching is not semantics"
            );
        }
    }
}
