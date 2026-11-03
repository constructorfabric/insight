//! The schema the squashed history leaves behind, as the statements that build
//! it from nothing.
//!
//! Every statement converges when re-run: the DDL is not transactional, so a
//! baseline interrupted part-way is finished by the next `migrate`.

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

pub const REQUIRED_TAG_CHECKS: &[&str] = &[
    "chk_metric_definition_tags_tag_shape",
    "chk_metric_definition_tags_display_order_nonnegative",
];

// INVARIANT: a referenced table is created before the tables that reference it,
// and a trigger after the tables it reads and writes.
pub const STATEMENTS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS saved_queries (
        id BINARY(16) NOT NULL PRIMARY KEY,
        insight_tenant_id BINARY(16) NOT NULL,
        name VARCHAR(255) NOT NULL,
        description TEXT NULL,
        `sql` TEXT NOT NULL,
        created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        KEY idx_saved_queries_tenant (insight_tenant_id)
    )",
    "CREATE TABLE IF NOT EXISTS feedback (
        id BINARY(16) NOT NULL PRIMARY KEY,
        insight_tenant_id BINARY(16) NOT NULL,
        person_id BINARY(16) NOT NULL,
        message TEXT NOT NULL,
        path VARCHAR(512) NOT NULL,
        created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        KEY idx_feedback_tenant_created (insight_tenant_id, created_at)
    )",
    "CREATE TABLE IF NOT EXISTS ai_context_entries (
        id BINARY(16) NOT NULL PRIMARY KEY,
        insight_tenant_id BINARY(16) NOT NULL,
        scope VARCHAR(16) NOT NULL,
        person_id BINARY(16) NULL,
        title VARCHAR(255) NOT NULL,
        body TEXT NOT NULL,
        created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        KEY idx_ai_context_entries_scope (insight_tenant_id, scope, person_id)
    )",
    "CREATE TABLE IF NOT EXISTS ai_credentials (
        insight_tenant_id BINARY(16) NOT NULL,
        person_id BINARY(16) NOT NULL,
        nonce VARBINARY(12) NOT NULL,
        ciphertext VARBINARY(512) NOT NULL,
        hint VARCHAR(4) NOT NULL,
        created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        PRIMARY KEY (insight_tenant_id, person_id)
    )",
    "CREATE TABLE IF NOT EXISTS ai_settings (
        insight_tenant_id BINARY(16) NOT NULL PRIMARY KEY,
        system_prompt TEXT NULL,
        updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
    )",
    "CREATE TABLE IF NOT EXISTS metric_sources (
        id BINARY(16) NOT NULL PRIMARY KEY,
        tenant_id BINARY(16) NULL,
        tenant_id_sentinel BINARY(16) GENERATED ALWAYS AS (COALESCE(tenant_id, 0x00000000000000000000000000000000)) STORED,
        source_key VARCHAR(128) NOT NULL,
        source_kind ENUM('managed_observation','custom_observation_sql') NOT NULL,
        source_ref VARCHAR(256) NOT NULL,
        observation_sql MEDIUMTEXT NULL,
        evidence_ref VARCHAR(256) NULL,
        origin ENUM('builtin','custom') NOT NULL,
        is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
        schema_status ENUM('ok','error','unchecked') NOT NULL DEFAULT 'unchecked',
        schema_checked_at DATETIME(3) NULL,
        schema_error_code VARCHAR(64) NULL,
        evidence_schema_status ENUM('ok','error','unchecked') NOT NULL DEFAULT 'unchecked',
        evidence_schema_checked_at DATETIME(3) NULL,
        evidence_schema_error_code VARCHAR(64) NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_metric_sources_tenant_key (tenant_id_sentinel, source_key),
        CONSTRAINT chk_metric_sources_source_key_shape CHECK (source_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_sources_schema_error_biconditional CHECK ((schema_status = 'error') = (schema_error_code IS NOT NULL)),
        CONSTRAINT chk_metric_sources_evidence_error_biconditional CHECK ((evidence_schema_status = 'error') = (evidence_schema_error_code IS NOT NULL)),
        CONSTRAINT chk_metric_sources_observation_sql_biconditional CHECK ((source_kind = 'custom_observation_sql') = (observation_sql IS NOT NULL)),
        CONSTRAINT chk_metric_sources_evidence_error_enum CHECK (evidence_schema_error_code IS NULL OR evidence_schema_error_code IN ('table_not_found','column_not_found','detail_key_not_found','dimension_not_covered','unknown')),
        CONSTRAINT chk_metric_sources_schema_error_enum CHECK (schema_error_code IS NULL OR schema_error_code IN ('table_not_found','column_not_found','detail_key_not_found','dimension_not_covered','unknown'))
    )",
    "CREATE TABLE IF NOT EXISTS metric_source_measures (
        id BINARY(16) NOT NULL PRIMARY KEY,
        source_id BINARY(16) NOT NULL,
        measure_key VARCHAR(128) NOT NULL,
        evidence_granularity ENUM('event','source_summary','derived_population') NULL,
        evidence_presentation JSON NULL,
        alias_collapse ENUM('sum','max','min') NOT NULL DEFAULT 'sum',
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
        CONSTRAINT chk_metric_source_measures_schema_error_enum CHECK (schema_error_code IS NULL OR schema_error_code IN ('table_not_found','column_not_found','detail_key_not_found','dimension_not_covered','unknown'))
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
        short_label VARCHAR(64) NULL,
        subject VARCHAR(64) NULL,
        description VARCHAR(2048) NULL,
        explanation VARCHAR(4096) NULL,
        unit VARCHAR(32) NULL,
        format ENUM('integer','decimal','currency','percent') NOT NULL,
        direction ENUM('higher_is_better','lower_is_better','neutral') NOT NULL,
        entity_type VARCHAR(64) NOT NULL,
        computation_type ENUM('sum','ratio','median','distinct_count','percentile','stddev') NOT NULL,
        scale DOUBLE NULL,
        denominator_aggregation ENUM('sum','distinct_count') NOT NULL DEFAULT 'sum',
        peer_cohort_key VARCHAR(64) NULL,
        origin ENUM('builtin','custom') NOT NULL,
        is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
        schema_status ENUM('ok','error','unchecked') NOT NULL DEFAULT 'unchecked',
        schema_checked_at DATETIME(3) NULL,
        schema_error_code VARCHAR(64) NULL,
        first_observed_date DATE NULL,
        last_observed_date DATE NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
        transform_multiplier DOUBLE NULL,
        transform_offset DOUBLE NULL,
        transform_clamp_min DOUBLE NULL,
        transform_clamp_max DOUBLE NULL,
        UNIQUE KEY uq_metric_definitions_tenant_key (tenant_id_sentinel, metric_key),
        KEY idx_metric_definitions_metric_key (metric_key),
        CONSTRAINT chk_metric_definitions_metric_key_shape CHECK (metric_key REGEXP BINARY '^[a-z][a-z0-9_]*[.][a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_definitions_entity_type_shape CHECK (entity_type REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_definitions_peer_cohort_key_shape CHECK (peer_cohort_key IS NULL OR peer_cohort_key REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_definitions_schema_error_biconditional CHECK ((schema_status = 'error') = (schema_error_code IS NOT NULL)),
        CONSTRAINT chk_metric_definitions_computation_fields CHECK (
            (computation_type = 'sum' AND scale IS NULL)
            OR (computation_type = 'ratio' AND scale IS NOT NULL)
            OR (computation_type = 'median' AND scale IS NULL)
            OR (computation_type = 'distinct_count' AND scale IS NULL)
            OR (computation_type = 'percentile' AND scale IS NOT NULL AND scale >= 0 AND scale <= 1)
            OR (computation_type = 'stddev' AND scale IS NULL)
        ),
        CONSTRAINT chk_metric_definitions_schema_error_enum CHECK (schema_error_code IS NULL OR schema_error_code IN ('table_not_found','column_not_found','detail_key_not_found','dimension_not_covered','unknown'))
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
    "CREATE TABLE IF NOT EXISTS metric_definition_tags (
        id BINARY(16) NOT NULL PRIMARY KEY,
        metric_definition_id BINARY(16) NOT NULL,
        tag VARCHAR(64) NOT NULL,
        display_order INT NOT NULL,
        created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
        UNIQUE KEY uq_metric_definition_tags_tag (metric_definition_id, tag),
        CONSTRAINT fk_metric_definition_tags_definition FOREIGN KEY (metric_definition_id) REFERENCES metric_definitions(id) ON DELETE CASCADE,
        CONSTRAINT chk_metric_definition_tags_tag_shape CHECK (tag REGEXP BINARY '^[a-z][a-z0-9_]*$'),
        CONSTRAINT chk_metric_definition_tags_display_order_nonnegative CHECK (display_order >= 0)
    )",
    // An edit to what a source or measure claims about its evidence unchecks the
    // source, so the next sweep re-probes it.
    // WORKAROUND: MariaDB stores `IF NOT EXISTS` in a trigger's definition, so a
    // trigger converges by drop-and-create instead.
    "DROP TRIGGER IF EXISTS trg_metric_sources_evidence_ref_invalidate",
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
    "DROP TRIGGER IF EXISTS trg_metric_source_measures_evidence_insert",
    "CREATE TRIGGER trg_metric_source_measures_evidence_insert
             AFTER INSERT ON metric_source_measures
             FOR EACH ROW
             UPDATE metric_sources
             SET evidence_schema_status = 'unchecked',
                 evidence_schema_checked_at = NULL,
                 evidence_schema_error_code = NULL
             WHERE id = NEW.source_id",
    "DROP TRIGGER IF EXISTS trg_metric_source_measures_evidence_update",
    "CREATE TRIGGER trg_metric_source_measures_evidence_update
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
     END",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::error_code::ALL_METRIC_SCHEMA_ERROR_CODES;
    use crate::migration::{ai_assist_schema, feedback_schema};

    fn statement_for(table: &str) -> &'static str {
        let head = format!("CREATE TABLE IF NOT EXISTS {table} (");
        let Some(statement) = STATEMENTS.iter().find(|s| s.starts_with(&head)) else {
            panic!("no CREATE TABLE for {table}");
        };
        statement
    }

    #[test]
    fn every_schema_error_check_accepts_the_whole_error_code_enum() {
        let codes = ALL_METRIC_SCHEMA_ERROR_CODES
            .iter()
            .map(|code| format!("'{}'", code.as_db()))
            .collect::<Vec<_>>()
            .join(",");

        let columns = [
            ("metric_sources", "schema_error_code"),
            ("metric_sources", "evidence_schema_error_code"),
            ("metric_source_measures", "schema_error_code"),
            ("metric_definitions", "schema_error_code"),
        ];
        for (table, column) in columns {
            let clause = format!("{column} IS NULL OR {column} IN ({codes})");
            assert!(
                statement_for(table).contains(&clause),
                "{table}.{column} CHECK out of sync with MetricSchemaErrorCode"
            );
        }
    }

    #[test]
    fn every_required_check_is_created() {
        let all_sql = STATEMENTS.join("\n");
        for check in REQUIRED_SOURCE_CHECKS
            .iter()
            .chain(REQUIRED_SOURCE_MEASURE_CHECKS)
            .chain(REQUIRED_SOURCE_DIMENSION_CHECKS)
            .chain(REQUIRED_DEFINITION_CHECKS)
            .chain(REQUIRED_DIMENSION_CHECKS)
            .chain(REQUIRED_TAG_CHECKS)
        {
            assert!(
                all_sql.contains(&format!("CONSTRAINT {check} CHECK")),
                "missing CHECK {check}"
            );
        }
    }

    #[test]
    fn a_referenced_table_is_created_before_its_referrers() {
        for (index, statement) in STATEMENTS.iter().enumerate() {
            for reference in statement.split("REFERENCES ").skip(1) {
                let parent = reference.split('(').next().unwrap_or_default();
                assert!(
                    STATEMENTS[..index]
                        .iter()
                        .any(|s| s.starts_with(&format!("CREATE TABLE IF NOT EXISTS {parent} ("))),
                    "{parent} must be created before the statement at {index}"
                );
            }
        }
    }

    #[test]
    fn an_evidence_edit_unchecks_the_source() {
        let trigger = |name: &str| {
            let head = format!("CREATE TRIGGER {name}\n");
            let Some(statement) = STATEMENTS.iter().find(|s| s.starts_with(&head)) else {
                panic!("no CREATE TRIGGER for {name}");
            };
            *statement
        };

        let cases = [
            (
                "trg_metric_sources_evidence_ref_invalidate",
                &[
                    "OLD.evidence_ref <=> NEW.evidence_ref",
                    "OLD.source_ref <=> NEW.source_ref",
                ][..],
            ),
            ("trg_metric_source_measures_evidence_insert", &[][..]),
            (
                "trg_metric_source_measures_evidence_update",
                &[
                    "OLD.evidence_granularity <=> NEW.evidence_granularity",
                    "OLD.evidence_presentation <=> NEW.evidence_presentation",
                ][..],
            ),
        ];
        for (name, conditions) in cases {
            let body = trigger(name);
            assert!(
                body.contains("evidence_schema_status = 'unchecked'"),
                "{name} should uncheck the source"
            );
            for condition in conditions {
                assert!(
                    body.contains(condition),
                    "{name} should react to {condition}"
                );
            }
        }
    }

    #[test]
    fn every_statement_converges_when_re_run() {
        for (index, statement) in STATEMENTS.iter().enumerate() {
            if statement.starts_with("CREATE TABLE IF NOT EXISTS ") {
                continue;
            }
            let Some(trigger) = statement
                .strip_prefix("CREATE TRIGGER ")
                .and_then(|rest| rest.split_whitespace().next())
            else {
                assert!(
                    statement.starts_with("DROP TRIGGER IF EXISTS "),
                    "should be safe to re-run: {statement}"
                );
                continue;
            };
            assert_eq!(
                index.checked_sub(1).map(|previous| STATEMENTS[previous]),
                Some(format!("DROP TRIGGER IF EXISTS {trigger}").as_str()),
                "{trigger} should be dropped right before it is created"
            );
        }
    }

    #[test]
    fn column_widths_match_the_budgets_the_writers_clip_to() {
        let cases = [
            (
                "feedback",
                format!("path VARCHAR({})", feedback_schema::PATH),
            ),
            (
                "ai_context_entries",
                format!("title VARCHAR({})", ai_assist_schema::TITLE),
            ),
        ];
        for (table, column) in cases {
            assert!(
                statement_for(table).contains(&column),
                "{table} should declare {column}"
            );
        }
    }

    #[test]
    fn computation_type_enum_and_check_cover_every_kind() {
        let definitions = statement_for("metric_definitions");
        assert!(definitions.contains(
            "computation_type ENUM('sum','ratio','median','distinct_count','percentile','stddev')"
        ));
        for kind in [
            "sum",
            "ratio",
            "median",
            "distinct_count",
            "percentile",
            "stddev",
        ] {
            assert!(
                definitions.contains(&format!("(computation_type = '{kind}' AND scale IS")),
                "chk_metric_definitions_computation_fields should cover {kind}"
            );
        }
    }
}
