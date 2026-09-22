//! The `MariaDB` schema the definitions live in.
//!
//! Applied by the `migrate` subcommand, which compose runs once before the
//! server starts. Every script is idempotent (`CREATE TABLE IF NOT EXISTS`),
//! so a run against a schema that is already current only writes the
//! `seaql_migrations` ledger.
//!
//! INVARIANT: every migration names itself, and no two names are the same.
//! The ledger is keyed by that name, so two migrations sharing one would let
//! the ledger say the second had run when it had not.

use sea_orm::{DatabaseConnection, Statement};
use sea_orm_migration::prelude::*;

/// What the first migration was called before it was given a name of its own.
///
/// `DeriveMigrationName` took the file stem, so both migrations answered
/// `migration` and the ledger of an installation that ran the older build
/// holds that. See [`name_the_first_migration`].
const UNNAMED: &str = "migration";

/// The name the first migration answers to now.
const FIRST: &str = "m20260907_000001_definitions";

/// The table sea-orm keeps the ledger in.
const LEDGER: &str = "seaql_migrations";

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260907_000001_definitions::Migration),
            Box::new(m20260916_000002_datasets::Migration),
        ]
    }
}

/// Gives the first migration its name in a ledger that still calls it
/// `migration`.
///
/// SAFETY: an installation that ran the build before the migrations named
/// themselves holds a row this code no longer knows, and sea-orm refuses to
/// run at all against a ledger naming a migration it cannot find - so the
/// service would not start and the deploy would roll back. Renaming the row
/// says what is already true: that migration is the one whose tables are
/// there. Nothing is applied here, and a ledger that never held the old name
/// is left alone.
pub(crate) async fn name_the_first_migration(db: &DatabaseConnection) -> Result<(), DbErr> {
    // A database this service has never written has no ledger to read, which
    // is the same as having nothing to rename. Every other failure is reported:
    // a rename skipped because the database was briefly unreachable would leave
    // the migrator to abort on the name it cannot find, which reads as a
    // missing migration rather than as the database being down.
    if !SchemaManager::new(db).has_table(LEDGER).await? {
        return Ok(());
    }

    let backend = sea_orm::ConnectionTrait::get_database_backend(db);
    let held = Statement::from_sql_and_values(backend, holds(), [UNNAMED.into(), FIRST.into()]);
    let rows = sea_orm::ConnectionTrait::query_all_raw(db, held).await?;

    let mut names = Vec::with_capacity(rows.len());
    for row in &rows {
        names.push(row.try_get::<String>("", "version")?);
    }
    if !needs_naming(&names) {
        return Ok(());
    }

    let rename =
        Statement::from_sql_and_values(backend, RENAME_FIRST, [FIRST.into(), UNNAMED.into()]);
    sea_orm::ConnectionTrait::execute_raw(db, rename).await?;
    tracing::info!(
        from = UNNAMED,
        to = FIRST,
        "named the first migration in a ledger written before the names"
    );

    Ok(())
}

/// Whether the ledger calls the first migration by the name it no longer has.
///
/// Both names at once would leave two rows for one migration, so a ledger
/// holding the new name is left as it is.
fn needs_naming(held: &[String]) -> bool {
    held.iter().any(|name| name == UNNAMED) && !held.iter().any(|name| name == FIRST)
}

fn holds() -> &'static str {
    "SELECT version FROM seaql_migrations WHERE version IN (?, ?)"
}

const RENAME_FIRST: &str = "UPDATE seaql_migrations SET version = ? WHERE version = ?";

/// One statement per call is all the `MySQL` wire protocol takes, so a script
/// is applied a statement at a time.
async fn apply_sql(manager: &SchemaManager<'_>, script: &str) -> Result<(), DbErr> {
    let db = manager.get_connection();
    for statement in statements(script) {
        db.execute_unprepared(&statement).await?;
    }

    Ok(())
}

/// The statements a script holds, split on `;`.
///
/// INVARIANT: comments come off first. A `;` inside one would otherwise end a
/// statement halfway through, which no test of the script's text can see and
/// only a real server refuses. These scripts carry no string literals, so the
/// remaining semicolons are all statement ends.
fn statements(script: &str) -> Vec<String> {
    let bare: String = script
        .lines()
        .map(|line| match line.split_once("--") {
            Some((before, _)) => before,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");

    bare.split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod ledger_tests {
    use super::{FIRST, LEDGER, RENAME_FIRST, UNNAMED, holds, needs_naming};

    /// An installation that ran the build before the migrations named
    /// themselves: sea-orm refuses to run at all against a ledger naming a
    /// migration it cannot find, so the row is renamed to what it already is.
    #[test]
    fn a_ledger_written_before_the_names_needs_its_first_migration_named() {
        assert!(needs_naming(&[UNNAMED.to_owned()]));
    }

    #[test]
    fn a_ledger_that_already_names_it_is_left_alone() {
        let held = [FIRST.to_owned(), "m20260916_000002_datasets".to_owned()];

        assert!(!needs_naming(&held));
    }

    /// Both names at once would mean two rows for one migration, so the old
    /// one is not renamed over the new.
    #[test]
    fn a_ledger_holding_both_names_is_left_alone() {
        assert!(!needs_naming(&[UNNAMED.to_owned(), FIRST.to_owned()]));
    }

    #[test]
    fn a_ledger_with_neither_name_is_left_alone() {
        assert!(!needs_naming(&[]));
        assert!(!needs_naming(&["m20260916_000002_datasets".to_owned()]));
    }

    /// The rename touches one row by name and writes nothing else: a ledger
    /// is the only record of what has run.
    #[test]
    fn the_rename_names_one_row_and_nothing_more() {
        assert!(RENAME_FIRST.starts_with("UPDATE seaql_migrations SET version = ?"));
        assert!(RENAME_FIRST.ends_with("WHERE version = ?"));
        assert!(!RENAME_FIRST.contains("DELETE"), "{RENAME_FIRST}");
    }

    #[test]
    fn the_ledger_is_read_by_the_two_names_it_might_hold() {
        assert!(holds().contains(&format!("FROM {LEDGER}")));
        assert!(holds().contains("version IN (?, ?)"));
    }
}

mod m20260907_000001_definitions {
    use super::{DbErr, MigrationName, MigrationTrait, SchemaManager, apply_sql};

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &'static str {
            "m20260907_000001_definitions"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            apply_sql(manager, include_str!("sql/001_definitions.sql")).await
        }

        async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            Err(DbErr::Custom(
                "dropping the definitions would destroy every dashboard".to_owned(),
            ))
        }
    }
}

mod m20260916_000002_datasets {
    use super::{DbErr, MigrationName, MigrationTrait, SchemaManager, apply_sql};

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &'static str {
            "m20260916_000002_datasets"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            apply_sql(manager, include_str!("sql/002_datasets.sql")).await
        }

        async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            Err(DbErr::Custom(
                "dropping the datasets would orphan every record they hold".to_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_script_creates_a_table_per_kind_keyed_by_name() {
        let script = include_str!("sql/001_definitions.sql");

        for table in ["metrics", "widgets", "dashboards"] {
            assert!(
                script.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
                "{table} is missing from the script"
            );
        }
        // The name is the key, which is what stops two rows claiming one name
        // and what makes a write an upsert rather than a second version.
        assert_eq!(
            script
                .matches("name VARCHAR(128) NOT NULL PRIMARY KEY")
                .count(),
            3
        );
        assert!(!script.contains("ReplacingMergeTree"));
    }

    /// The ledger is keyed by the name a migration gives itself. Two sharing
    /// one name collide on a fresh database and, worse, let an installation
    /// that already ran the first believe it has run the second.
    #[test]
    fn every_migration_names_itself_and_no_two_names_are_the_same() {
        use sea_orm_migration::MigratorTrait as _;

        let migrations = super::Migrator::migrations();
        let names: Vec<&str> = migrations
            .iter()
            .map(|migration| migration.name())
            .collect();

        assert_eq!(
            names,
            ["m20260907_000001_definitions", "m20260916_000002_datasets"],
            "a migration's name is the ledger's key, so it is written down here"
        );
    }

    /// Every script is applied a statement at a time, and a statement cut in
    /// half is a syntax error only a real server reports. Reading the text is
    /// not enough: the split is what the server sees.
    #[test]
    fn every_script_splits_into_whole_statements() {
        let scripts = [
            (
                "001_definitions.sql",
                include_str!("sql/001_definitions.sql"),
                3,
            ),
            ("002_datasets.sql", include_str!("sql/002_datasets.sql"), 1),
        ];

        for (named, script, expected) in scripts {
            let statements = super::statements(script);

            assert_eq!(
                statements.len(),
                expected,
                "{named} should hold {expected} statements: {statements:#?}"
            );
            for statement in &statements {
                assert!(
                    statement.starts_with("CREATE TABLE IF NOT EXISTS"),
                    "{named} holds a statement that is not a whole one: {statement}"
                );
                assert!(
                    statement.ends_with(')') || statement.contains("COLLATE=utf8mb4_unicode_ci"),
                    "{named} holds a statement that stops early: {statement}"
                );
            }
        }
    }

    #[test]
    fn a_dataset_row_carries_its_state_and_the_table_holding_its_records() {
        let script = include_str!("sql/002_datasets.sql");

        assert!(
            script.contains("CREATE TABLE IF NOT EXISTS datasets"),
            "{script}"
        );
        for column in [
            "name VARCHAR(128) NOT NULL PRIMARY KEY",
            "body JSON NOT NULL",
            "state VARCHAR(16) NOT NULL",
            "physical_table VARCHAR(128) NULL",
            "operation VARCHAR(16) NULL",
            "operation_token VARCHAR(64) NULL",
            "lease_until DATETIME(6) NULL",
        ] {
            assert!(
                script.contains(column),
                "{column} is missing from the script"
            );
        }
    }
}
