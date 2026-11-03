//! Database migrations for the identity-resolution service.
//!
//! Each module applies one `sql/NNN_*.sql` script through [`apply_sql`].
//! [`SQUASHED`] lists the history the baseline replaced: the names stay in the
//! migrator, applying nothing, so a ledger that walked that history still
//! matches this build (see [`insight_migration::squash`]).
//!
//! Timezone note: sqlx pins every pooled `MySQL` connection to
//! `time_zone='+00:00'`, so `SET time_zone` statements inside the scripts are
//! belt-and-braces, not load-bearing.

mod m20261014_000019_drop_dbup_ledger_lowercase;
mod m20261030_000001_baseline;

use insight_migration::squash;
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;

/// The migrations `m20261030_000001_baseline` squashed, in their original
/// order.
///
/// INVARIANT: never rename, reorder or drop an entry — databases that walked
/// the old history hold exactly these names in their ledger.
pub(crate) const SQUASHED: &[&str] = &[
    "m20260724_000001_persons",
    "m20260724_000002_account_person_map",
    "m20260724_000003_org_chart",
    "m20260724_000004_persons_relax_constraints",
    "m20260724_000005_tighten_source_type",
    "m20260724_000006_visibility",
    "m20260724_000007_roles",
    "m20260724_000008_person_roles",
    "m20260724_000009_align_existing_tables_to_conventions",
    "m20260724_000010_account_person_map_idx_by_account",
    "m20260724_000011_operations",
    "m20260724_000012_org_chart_nullable_parent",
    "m20260724_000013_persons_email_any_tenant_idx",
    "m20260724_000014_account_person_map_datetime",
    "m20260817_000015_drop_account_person_map",
    "m20260817_000016_drop_dbup_ledger",
    "m20260831_000017_previews_admin_role",
    "m20260901_000018_people",
];

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations: Vec<Box<dyn MigrationTrait>> =
            vec![Box::new(m20261030_000001_baseline::Migration)];
        migrations.extend(squash::squashed(SQUASHED));
        migrations.push(Box::new(
            m20261014_000019_drop_dbup_ledger_lowercase::Migration,
        ));
        migrations
    }
}

/// Execute every statement of a SQL script on the migration connection, in
/// order.
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
    use super::{Migrator, MigratorTrait, SQUASHED, split_statements};

    /// Statement counts per script. A count change means either an edit to an
    /// applied script leaked in or the splitter broke — both must be looked
    /// at, not auto-accepted.
    #[test]
    fn splits_every_script_into_the_expected_statement_count() {
        let cases: &[(&str, &str, usize)] = &[
            ("000_baseline", include_str!("sql/000_baseline.sql"), 8),
            (
                "019_drop_dbup_ledger_lowercase",
                include_str!("sql/019_drop_dbup_ledger_lowercase.sql"),
                1,
            ),
        ];
        for (name, script, expected) in cases {
            let stmts = split_statements(script);
            assert_eq!(
                stmts.len(),
                *expected,
                "script {name} split into {} statements, expected {expected}",
                stmts.len(),
            );
            for stmt in &stmts {
                assert!(
                    !stmt.contains("--"),
                    "{name}: comment leaked into statement"
                );
            }
        }
    }

    /// The seeded role ids are stable API; this pins them against edits.
    #[test]
    fn seeded_role_ids_and_names_are_stable() {
        use crate::infra::db::roles_repo::ADMIN_ROLE_ID;

        let script = include_str!("sql/000_baseline.sql");
        let admin_hex = ADMIN_ROLE_ID.simple().to_string();
        for (hex, name) in [
            (admin_hex.as_str(), "admin"),
            ("a4d11000000040008000000000000002", "previews-admin"),
        ] {
            assert!(
                script
                    .to_lowercase()
                    .contains(&format!("(unhex('{hex}'), '{name}')")),
                "{name}: the baseline must seed UNHEX('{hex}') under that name"
            );
        }
    }

    #[test]
    fn the_squashed_names_follow_the_baseline_and_precede_newer_migrations() {
        let names: Vec<String> = Migrator::migrations()
            .iter()
            .map(|migration| migration.name().to_owned())
            .collect();

        let mut expected = vec!["m20261030_000001_baseline".to_owned()];
        expected.extend(SQUASHED.iter().map(|name| (*name).to_owned()));
        expected.push("m20261014_000019_drop_dbup_ledger_lowercase".to_owned());
        assert_eq!(names, expected);
    }

    #[test]
    fn strips_comments_and_trailing_fragments() {
        let stmts = split_statements("-- header\nSELECT 1;\n-- tail\n");
        assert_eq!(stmts, vec!["SELECT 1".to_owned()]);
    }
}
