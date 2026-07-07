//! Create `metric_catalog` — product-owned semantic metadata for one `metric_key`.
//!
//! Refs #519. Schema source: `docs/domain/metric-catalog/specs/DESIGN.md` §3.7
//! (`cpt-metric-cat-dbtable-metric-catalog`).
//!
//! CHECK constraint names below are load-bearing: the startup probe in
//! `infra/db/check_probe.rs` looks them up in `INFORMATION_SCHEMA.CHECK_CONSTRAINTS`
//! and refuses to boot if any are missing (closes the MariaDB 10.2 silent-drop
//! hole per `cpt-metric-cat-constraint-mariadb-check`). Keep this list in sync
//! with [`REQUIRED_CHECKS`].

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Names of every CHECK constraint this migration adds. The startup probe
/// asserts each one is present in `INFORMATION_SCHEMA.CHECK_CONSTRAINTS`.
pub const REQUIRED_CHECKS: &[&str] = &[
    "chk_metric_catalog_tenant_id_null",
    "chk_metric_catalog_metric_key_shape",
    "chk_metric_catalog_schema_status_enum",
    "chk_metric_catalog_schema_error_biconditional",
    "chk_metric_catalog_schema_error_enum",
];

/// CHECK clause SQL, ordered to match [`REQUIRED_CHECKS`]. Each is appended
/// via `ALTER TABLE ... ADD CONSTRAINT <name> CHECK (<predicate>)`.
const CHECK_CLAUSES: &[(&str, &str)] = &[
    ("chk_metric_catalog_tenant_id_null", "tenant_id IS NULL"),
    // Dot is expressed as a `[.]` character class rather than `\.` so the
    // predicate is independent of MariaDB's sql_mode. With sql_mode default,
    // `'\\.'` collapses to `\.` (literal dot — correct); with
    // `NO_BACKSLASH_ESCAPES` set, the two backslashes survive into the regex
    // and `\\.` would silently start meaning "backslash + any char", rejecting
    // every legitimate metric_key. `[.]` sidesteps the whole escape question.
    //
    // `REGEXP BINARY` forces case-sensitive matching against the lowercase
    // class `[a-z]`. MariaDB 11.x defaults to a case-insensitive
    // `utf8mb4_uca1400_ai_ci` collation, under which `[a-z]` silently matches
    // uppercase too — making `UPPER.case` pass a check meant to reject it.
    // `BINARY` compares bytes, so `[a-z]` is the byte range 0x61..0x7a only.
    (
        "chk_metric_catalog_metric_key_shape",
        "metric_key REGEXP BINARY '^[a-z][a-z0-9_]*[.][a-z][a-z0-9_]*$'",
    ),
    (
        "chk_metric_catalog_schema_status_enum",
        "schema_status IN ('ok','error','unchecked')",
    ),
    (
        "chk_metric_catalog_schema_error_biconditional",
        "(schema_status = 'error') = (schema_error_code IS NOT NULL)",
    ),
    (
        "chk_metric_catalog_schema_error_enum",
        "schema_error_code IS NULL OR schema_error_code IN ('table_not_found','column_not_found','clickhouse_unreachable','unknown')",
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    #[allow(clippy::too_many_lines)] // single-table DDL — splitting hurts readability
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MetricCatalog::Table)
                    .if_not_exists()
                    // UUIDv7 stored as BINARY(16) — ~4× smaller index pages than
                    // CHAR(36) and time-ordered for insert locality. See DESIGN §3.7
                    // PK note + §4 oq-pk-strategy option γ.
                    .col(
                        ColumnDef::new(MetricCatalog::Id)
                            .binary_len(16)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::TenantId)
                            .binary_len(16)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::MetricKey)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::Label)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::Sublabel)
                            .string_len(128)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::Description)
                            .string_len(2048)
                            .null(),
                    )
                    .col(ColumnDef::new(MetricCatalog::Unit).string_len(32).null())
                    .col(ColumnDef::new(MetricCatalog::Format).string_len(32).null())
                    .col(
                        ColumnDef::new(MetricCatalog::HigherIsBetter)
                            .boolean()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::IsMemberScale)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(MetricCatalog::SourceTags).json().not_null())
                    .col(
                        ColumnDef::new(MetricCatalog::IsEnabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::SchemaStatus)
                            .custom(Alias::new("ENUM('ok','error','unchecked')"))
                            .not_null()
                            .default("unchecked"),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::SchemaCheckedAt)
                            .timestamp()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::SchemaErrorCode)
                            .string_len(32)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(MetricCatalog::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp())
                            .extra("ON UPDATE CURRENT_TIMESTAMP"),
                    )
                    .to_owned(),
            )
            .await?;

        // UNIQUE on metric_key — doubles as the resolver's lookup index when
        // threshold-resolver joins metric_threshold by metric_key (§3.7 line 987).
        manager
            .create_index(
                Index::create()
                    .name("uq_metric_catalog_metric_key")
                    .table(MetricCatalog::Table)
                    .col(MetricCatalog::MetricKey)
                    .unique()
                    .to_owned(),
            )
            .await?;

        let conn = manager.get_connection();
        // `{name}` is backtick-quoted defensively. Both `name` and
        // `predicate` come from the compile-time `CHECK_CLAUSES` const above
        // — no injection vector today — but the backticks make a future
        // entry whose name happens to be a MariaDB reserved word still parse.
        for (name, predicate) in CHECK_CLAUSES {
            conn.execute_unprepared(&format!(
                "ALTER TABLE metric_catalog ADD CONSTRAINT `{name}` CHECK ({predicate})"
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // we have only forward migrations
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[derive(DeriveIden)]
enum MetricCatalog {
    Table,
    Id,
    TenantId,
    MetricKey,
    Label,
    Sublabel,
    Description,
    Unit,
    Format,
    HigherIsBetter,
    IsMemberScale,
    SourceTags,
    IsEnabled,
    SchemaStatus,
    SchemaCheckedAt,
    SchemaErrorCode,
    CreatedAt,
    UpdatedAt,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_clauses_match_required_list() {
        let clause_names: Vec<&str> = CHECK_CLAUSES.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            clause_names.as_slice(),
            REQUIRED_CHECKS,
            "CHECK_CLAUSES names must equal REQUIRED_CHECKS in the same order — \
             the startup probe relies on this list to detect drops"
        );
    }

    fn predicate_for(name: &str) -> &'static str {
        let Some((_, p)) = CHECK_CLAUSES.iter().find(|(n, _)| *n == name) else {
            panic!("CHECK {name} not registered in CHECK_CLAUSES");
        };
        p
    }

    #[test]
    fn metric_key_regex_predicate_is_verbatim() {
        // Regression guard against silent typos in the metric_key shape regex.
        // Shape: "table_name.column_name", lowercase snake_case both sides.
        // See DESIGN.md §3.7 line 976. Dot is `[.]` (sql_mode-independent).
        let predicate = predicate_for("chk_metric_catalog_metric_key_shape");
        assert!(predicate.contains("^[a-z][a-z0-9_]*"));
        assert!(
            predicate.contains("[.]"),
            "dot separator must be a character class, not `\\.` (sql_mode brittleness)"
        );
        assert!(
            !predicate.contains(r"\\."),
            "do NOT use `\\.` — collapses unsafely under sql_mode=NO_BACKSLASH_ESCAPES"
        );
        assert!(predicate.contains("[a-z0-9_]*$"));
        // `REGEXP BINARY` enforces case-sensitivity; MariaDB 11.x defaults to
        // a case-insensitive collation under which `[a-z]` silently matches
        // uppercase. Without `BINARY`, `UPPER.case` passes the check.
        assert!(
            predicate.contains("REGEXP BINARY"),
            "CHECK must use `REGEXP BINARY` for case-sensitive matching on \
             MariaDB 11+ collations; got: {predicate}"
        );
    }

    #[test]
    fn schema_status_enum_predicate_lists_all_three() {
        let predicate = predicate_for("chk_metric_catalog_schema_status_enum");
        for v in ["'ok'", "'error'", "'unchecked'"] {
            assert!(
                predicate.contains(v),
                "schema_status enum CHECK missing value {v}"
            );
        }
    }

    #[test]
    fn schema_error_code_enum_predicate_lists_all_four() {
        let predicate = predicate_for("chk_metric_catalog_schema_error_enum");
        for v in [
            "'table_not_found'",
            "'column_not_found'",
            "'clickhouse_unreachable'",
            "'unknown'",
        ] {
            assert!(
                predicate.contains(v),
                "schema_error_code enum CHECK missing value {v}"
            );
        }
    }
}
