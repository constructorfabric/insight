//! The row shape a coverage window was built at.
//!
//! A measure's row kind is decided by its own fold and by whether any metric
//! reads it as a distribution, so a release can change it without moving the
//! measure's `definition_version`. Coverage that cannot state the shape it
//! attests cannot keep a read from folding two shapes into one number.

use sea_orm_migration::prelude::*;

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

// INVARIANT: the column takes no default, so every coverage row states the
// shape it was built at. Rows written before it existed cannot, and a refresh
// re-earns them rather than a backfill inventing a shape they may not have.
const SCHEMA_STATEMENTS: &[&str] = &[
    "DELETE FROM semantic_cache_coverage",
    "ALTER TABLE semantic_cache_coverage
        ADD COLUMN row_kind ENUM('aggregate', 'event', 'subject') NOT NULL
        AFTER definition_version",
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_column_admits_exactly_the_three_row_shapes_a_build_writes() {
        let all_sql = SCHEMA_STATEMENTS.join("\n");

        assert!(
            all_sql.contains("ENUM('aggregate', 'event', 'subject')"),
            "{all_sql}"
        );
        assert!(all_sql.contains("NOT NULL"), "{all_sql}");
        assert!(
            !all_sql.contains("DEFAULT"),
            "a default would let a coverage row claim a shape it was not built at: {all_sql}"
        );
    }

    #[test]
    fn coverage_written_before_the_column_existed_is_cleared_rather_than_backfilled() {
        assert_eq!(SCHEMA_STATEMENTS[0], "DELETE FROM semantic_cache_coverage");
    }

    #[tokio::test]
    async fn the_migration_is_forward_only() {
        let error = Migration
            .down(&SchemaManager::new(&sea_orm::DatabaseConnection::default()))
            .await
            .expect_err("a down migration is refused");

        assert!(matches!(error, DbErr::Custom(_)), "{error}");
    }
}
