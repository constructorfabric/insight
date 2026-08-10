//! Stores the inline custom observation SQL on a custom source.
//!
//! `source_ref` is `VARCHAR(256)` — the managed relation name. A custom source
//! (`source_kind='custom_observation_sql'`) instead carries an arbitrarily long
//! `SELECT` that emits the observation contract, so it needs its own wide
//! column. The biconditional CHECK ties the two together: a custom source has
//! SQL, a managed source has none — the invalid combinations are unstorable.

use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;

pub const OBSERVATION_SQL_CHECK: &str = "chk_metric_sources_observation_sql_biconditional";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        if !manager
            .has_column("metric_sources", "observation_sql")
            .await?
        {
            conn.execute_unprepared(ADD_COLUMN).await?;
        }

        conn.execute_unprepared(&format!(
            "ALTER TABLE metric_sources DROP CONSTRAINT IF EXISTS {OBSERVATION_SQL_CHECK}"
        ))
        .await?;
        conn.execute_unprepared(ADD_CHECK).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

const ADD_COLUMN: &str =
    "ALTER TABLE metric_sources ADD COLUMN observation_sql MEDIUMTEXT NULL AFTER source_ref";

const ADD_CHECK: &str = "ALTER TABLE metric_sources \
     ADD CONSTRAINT chk_metric_sources_observation_sql_biconditional CHECK ( \
        (source_kind = 'custom_observation_sql') = (observation_sql IS NOT NULL) \
     )";

#[cfg(test)]
mod tests {
    use super::*;

    /// The column is `MEDIUMTEXT NULL` and the biconditional CHECK carries the
    /// name the drop/re-add pair references, so the constraint is replaced in
    /// place on a warm cluster rather than duplicated.
    #[test]
    fn ddl_pins_column_and_biconditional_check() {
        assert!(ADD_COLUMN.contains("observation_sql MEDIUMTEXT NULL"));
        assert!(ADD_CHECK.contains(OBSERVATION_SQL_CHECK));
        assert!(
            ADD_CHECK.contains(
                "(source_kind = 'custom_observation_sql') = (observation_sql IS NOT NULL)"
            )
        );
    }
}
