//! The `MariaDB` schema the definitions live in.
//!
//! Applied by the `migrate` subcommand, which compose runs once before the
//! server starts. The script is idempotent (`CREATE TABLE IF NOT EXISTS`), so
//! a run against a schema that is already current only writes the
//! `seaql_migrations` ledger.

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20260907_000001_definitions::Migration)]
    }
}

/// One statement per call is all the `MySQL` wire protocol takes, so the
/// script is split on `;`. Safe for this script: no string literal in it
/// contains a semicolon.
async fn apply_sql(manager: &SchemaManager<'_>, script: &str) -> Result<(), DbErr> {
    let db = manager.get_connection();
    for statement in script
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        db.execute_unprepared(statement).await?;
    }

    Ok(())
}

mod m20260907_000001_definitions {
    use super::{DbErr, MigrationTrait, SchemaManager, apply_sql};
    use sea_orm_migration::prelude::DeriveMigrationName;

    #[derive(DeriveMigrationName)]
    pub struct Migration;

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
}
