//! Database migrations for the identity-resolution service.
//!
//! Ownership transfer (epic #1602 cutover): the `identity` MariaDB schema was
//! owned by the .NET identity service, which applied `sql/*.sql` via `DbUp` at
//! startup. The .NET service is FROZEN (no new migrations there) and will be
//! decommissioned once the Rust service is validated — from here on, schema
//! changes are authored HERE, as new `mNNN_*` modules.
//!
//! `sql/001…013` are byte-for-byte copies of the `DbUp` scripts
//! (`services/identity/src/Insight.Identity.Infrastructure/Migrations/`) —
//! review parity with `diff -r` — with ONE deliberate edit: `012`'s
//! constraint DROP/ADD are `IF EXISTS`/`IF NOT EXISTS` guarded (crash- and
//! concurrency-recovery; documented in the file). `014` is the first
//! Rust-authored migration (deliberately NOT added to the frozen .NET set —
//! one applier, no concurrent-ALTER window). Every script is idempotent
//! (`CREATE … IF NOT
//! EXISTS`, `MODIFY COLUMN` to the same type is a no-op, index/constraint
//! drops recreate under the same name), so the first Rust `migrate` run on an
//! environment whose schema `DbUp` already applied passes through as a no-op
//! sweep that only populates the `seaql_migrations` ledger. `DbUp`'s own
//! `SchemaVersions` ledger is left in place, orphaned, until the .NET service
//! is deleted.
//!
//! ROLLBACK POLICY (until .NET decommission): additive-only. While rolling
//! traffic back to the .NET service remains a supported escape hatch, no
//! migration here may drop/rename anything the .NET code reads or writes.
//!
//! Timezone note: sqlx pins every pooled `MySQL` connection to
//! `time_zone='+00:00'`, so `SET time_zone` statements inside the scripts are
//! belt-and-braces, not load-bearing.

mod m20260724_000001_persons;
mod m20260724_000002_account_person_map;
mod m20260724_000003_org_chart;
mod m20260724_000004_persons_relax_constraints;
mod m20260724_000005_tighten_source_type;
mod m20260724_000006_visibility;
mod m20260724_000007_roles;
mod m20260724_000008_person_roles;
mod m20260724_000009_align_existing_tables_to_conventions;
mod m20260724_000010_account_person_map_idx_by_account;
mod m20260724_000011_operations;
mod m20260724_000012_org_chart_nullable_parent;
mod m20260724_000013_persons_email_any_tenant_idx;
mod m20260724_000014_account_person_map_datetime;

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260724_000001_persons::Migration),
            Box::new(m20260724_000002_account_person_map::Migration),
            Box::new(m20260724_000003_org_chart::Migration),
            Box::new(m20260724_000004_persons_relax_constraints::Migration),
            Box::new(m20260724_000005_tighten_source_type::Migration),
            Box::new(m20260724_000006_visibility::Migration),
            Box::new(m20260724_000007_roles::Migration),
            Box::new(m20260724_000008_person_roles::Migration),
            Box::new(m20260724_000009_align_existing_tables_to_conventions::Migration),
            Box::new(m20260724_000010_account_person_map_idx_by_account::Migration),
            Box::new(m20260724_000011_operations::Migration),
            Box::new(m20260724_000012_org_chart_nullable_parent::Migration),
            Box::new(m20260724_000013_persons_email_any_tenant_idx::Migration),
            Box::new(m20260724_000014_account_person_map_datetime::Migration),
        ]
    }
}

/// Execute every statement of a `DbUp`-style SQL script on the migration
/// connection, in order.
///
/// The `MySQL` wire protocol takes one statement per call, so the script is
/// split on `;` after stripping `--` line comments. That is safe for these
/// scripts because none of their string literals contain a semicolon (the
/// splitter test below guards the statement counts).
pub(crate) async fn apply_sql(manager: &SchemaManager<'_>, script: &str) -> Result<(), DbErr> {
    let db = manager.get_connection();
    for stmt in split_statements(script) {
        db.execute_unprepared(&stmt).await?;
    }
    Ok(())
}

/// Strip `--` line comments, then split on `;`. Empty fragments (trailing
/// newline after the last `;`) are dropped.
fn split_statements(script: &str) -> Vec<String> {
    let no_comments: String = script
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    no_comments
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Shared irreversibility error for `down()`: the identity schema is an
/// append-only observation log; there is no supported downgrade path.
pub(crate) fn irreversible() -> DbErr {
    DbErr::Migration(
        "identity schema migrations are irreversible (append-only observation log)".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::split_statements;

    /// Statement counts per script. A count change means either an upstream
    /// .NET script edit leaked in (they are frozen) or the splitter broke —
    /// both must be looked at, not auto-accepted.
    #[test]
    fn splits_every_script_into_the_expected_statement_count() {
        let cases: &[(&str, usize)] = &[
            (include_str!("sql/001_persons.sql"), 1),
            (include_str!("sql/002_account_person_map.sql"), 1),
            (include_str!("sql/003_org_chart.sql"), 1),
            (include_str!("sql/004_persons_relax_constraints.sql"), 3),
            (include_str!("sql/005_tighten_source_type.sql"), 2),
            (include_str!("sql/006_visibility.sql"), 1),
            (include_str!("sql/007_roles.sql"), 2),
            (include_str!("sql/008_person_roles.sql"), 1),
            (
                include_str!("sql/009_align_existing_tables_to_conventions.sql"),
                2,
            ),
            (
                include_str!("sql/010_account_person_map_idx_by_account.sql"),
                1,
            ),
            (include_str!("sql/011_operations.sql"), 1),
            (include_str!("sql/012_org_chart_nullable_parent.sql"), 3),
            (include_str!("sql/013_persons_email_any_tenant_idx.sql"), 1),
            (include_str!("sql/014_account_person_map_datetime.sql"), 2),
        ];
        for (i, (script, expected)) in cases.iter().enumerate() {
            let stmts = split_statements(script);
            assert_eq!(
                stmts.len(),
                *expected,
                "script {:03} split into {} statements, expected {expected}",
                i + 1,
                stmts.len(),
            );
            for stmt in &stmts {
                assert!(!stmt.contains("--"), "comment leaked into statement");
            }
        }
    }

    #[test]
    fn strips_comments_and_trailing_fragments() {
        let stmts = split_statements("-- header\nSELECT 1;\n-- tail\n");
        assert_eq!(stmts, vec!["SELECT 1".to_owned()]);
    }
}
