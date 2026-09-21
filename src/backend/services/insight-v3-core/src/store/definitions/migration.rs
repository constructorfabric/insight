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

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260907_000001_definitions::Migration),
            Box::new(m20260916_000002_datasets::Migration),
            Box::new(m20260921_000003_dataset_source::Migration),
        ]
    }
}

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
/// only a real server refuses. No script here holds a `;` inside a string
/// literal either, so the remaining semicolons are all statement ends.
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
            [
                "m20260907_000001_definitions",
                "m20260916_000002_datasets",
                "m20260921_000003_dataset_source",
            ],
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

mod m20260921_000003_dataset_source {
    use super::{DbErr, MigrationName, MigrationTrait, SchemaManager, apply_sql};

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &'static str {
            "m20260921_000003_dataset_source"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            apply_sql(manager, include_str!("sql/003_dataset_source.sql")).await
        }

        /// Taking the property away again would leave every declaration
        /// unreadable by the service that wrote it.
        async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            Err(DbErr::Custom(
                "a declaration that does not say what it is over cannot be read".to_owned(),
            ))
        }
    }
}
