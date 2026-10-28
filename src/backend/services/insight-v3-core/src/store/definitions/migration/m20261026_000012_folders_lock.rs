//! A row that folder creations and hand-overs take turns on, so no owner's
//! folders pass the cap between a count and the write it allows.
//!
//! INVARIANT: every step is safe to run again.

use sea_orm::DbErr;
use sea_orm_migration::prelude::*;

const STEPS: [&str; 2] = [
    "CREATE TABLE IF NOT EXISTS folders_lock (
    id TINYINT NOT NULL PRIMARY KEY
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
    "INSERT IGNORE INTO folders_lock (id) VALUES (1)",
];

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261026_000012_folders_lock"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for step in STEPS {
            db.execute_unprepared(step).await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "dropping the folders lock would let an owner's folders pass the cap".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::STEPS;

    #[test]
    fn the_lock_row_is_made_once_and_runs_again_without_change() {
        assert!(
            STEPS[0].starts_with("CREATE TABLE IF NOT EXISTS folders_lock"),
            "{}",
            STEPS[0]
        );
        assert_eq!(STEPS[1], "INSERT IGNORE INTO folders_lock (id) VALUES (1)");
    }
}
