//! Metrics, widgets and dashboards are keyed by an id, and carry a name
//! beside it that two of them may share.
//!
//! The column that was the name becomes the id, keeping the primary key and
//! every foreign key that points at it, and the new name starts as the old
//! one. An older name already satisfies the id rule, so every reference to
//! one - in a body, a pin, a tag link or an alert - already holds a valid id,
//! and nothing else is rewritten.
//!
//! INVARIANT: every step is safe to run again. The rename runs only while
//! the table has a `name` and no `id`, the column and the index are added
//! with `IF NOT EXISTS`, the fill touches only rows with no name, and
//! dropping a default that is already gone changes nothing.

use sea_orm::{ConnectionTrait as _, DbBackend, DbErr, FromQueryResult, Statement};
use sea_orm_migration::prelude::*;

const TABLES: [&str; 3] = ["metrics", "widgets", "dashboards"];

const GLOBAL_SQL_MODE: &str = "SELECT @@GLOBAL.sql_mode AS mode";

const STRICT_MODES: [&str; 2] = ["STRICT_TRANS_TABLES", "STRICT_ALL_TABLES"];

const RENAME_KEY: &str = "ALTER TABLE {table} RENAME COLUMN name TO id";

/// The same collation as the tables themselves, so a name compares and
/// sorts the way the rest of the row does.
const ADD_NAME: &str = "ALTER TABLE {table} ADD COLUMN IF NOT EXISTS name VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL DEFAULT '' AFTER id";

const FILL_NAME: &str = "UPDATE {table} SET name = id WHERE name = ''";

const DROP_NAME_DEFAULT: &str = "ALTER TABLE {table} ALTER COLUMN name DROP DEFAULT";

const INDEX_NAME: &str = "CREATE INDEX IF NOT EXISTS {table}_name ON {table} (name)";

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261021_000009_definition_ids"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        refuse_outside_strict_mode(manager).await?;

        for table in TABLES {
            key_by_id(manager, table).await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "a name two definitions share cannot become a key again".to_owned(),
        ))
    }
}

#[derive(Debug, FromQueryResult)]
struct ModeRow {
    mode: String,
}

/// SAFETY: the previous build may still be serving while this runs, and it
/// writes a row naming no id. Only strict mode makes that write fail rather
/// than store a row whose id is an empty string.
async fn refuse_outside_strict_mode(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let held =
        ModeRow::find_by_statement(Statement::from_string(DbBackend::MySql, GLOBAL_SQL_MODE))
            .one(manager.get_connection())
            .await?;
    let mode = held.map(|row| row.mode).unwrap_or_default();

    if is_strict(&mode) {
        return Ok(());
    }

    Err(DbErr::Custom(format!(
        "definition ids need the server in strict mode (STRICT_TRANS_TABLES or STRICT_ALL_TABLES); sql_mode is `{mode}`"
    )))
}

fn is_strict(mode: &str) -> bool {
    mode.split(',').map(str::trim).any(|flag| {
        STRICT_MODES
            .iter()
            .any(|strict| flag.eq_ignore_ascii_case(strict))
    })
}

/// Moves one table from a name key to an id key with a name beside it.
///
/// `table` is one of [`TABLES`], or a scratch table a live test made.
pub(crate) async fn key_by_id(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
    let named = manager.has_column(table, "name").await?;
    let keyed = manager.has_column(table, "id").await?;
    let db = manager.get_connection();

    if named && !keyed {
        db.execute_unprepared(&for_table(RENAME_KEY, table)).await?;
    }

    for step in [ADD_NAME, FILL_NAME, DROP_NAME_DEFAULT, INDEX_NAME] {
        db.execute_unprepared(&for_table(step, table)).await?;
    }

    Ok(())
}

fn for_table(template: &str, table: &str) -> String {
    template.replace("{table}", table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_strict_server_is_migrated() {
        let cases = [
            ("STRICT_TRANS_TABLES", true),
            ("STRICT_ALL_TABLES", true),
            (
                "STRICT_TRANS_TABLES,ERROR_FOR_DIVISION_BY_ZERO,NO_AUTO_CREATE_USER",
                true,
            ),
            ("NO_ENGINE_SUBSTITUTION,STRICT_ALL_TABLES", true),
            ("strict_trans_tables", true),
            ("", false),
            ("NO_ENGINE_SUBSTITUTION", false),
            ("ERROR_FOR_DIVISION_BY_ZERO,NO_AUTO_CREATE_USER", false),
        ];

        for (mode, strict) in cases {
            assert_eq!(is_strict(mode), strict, "for sql_mode {mode:?}");
        }
    }

    #[test]
    fn every_step_is_one_that_runs_again_without_change() {
        assert!(
            ADD_NAME.contains("ADD COLUMN IF NOT EXISTS name"),
            "{ADD_NAME}"
        );
        assert!(
            ADD_NAME.contains("COLLATE utf8mb4_unicode_ci"),
            "{ADD_NAME}"
        );
        assert!(ADD_NAME.ends_with("AFTER id"), "{ADD_NAME}");
        assert!(FILL_NAME.ends_with("WHERE name = ''"), "{FILL_NAME}");
        assert!(
            INDEX_NAME.starts_with("CREATE INDEX IF NOT EXISTS"),
            "{INDEX_NAME}"
        );
        assert_eq!(
            for_table(RENAME_KEY, "widgets"),
            "ALTER TABLE widgets RENAME COLUMN name TO id"
        );
    }

    #[test]
    fn the_name_index_is_named_after_its_table() {
        assert_eq!(
            for_table(INDEX_NAME, "dashboards"),
            "CREATE INDEX IF NOT EXISTS dashboards_name ON dashboards (name)"
        );
    }
}
