//! Adds `definition_version` to `semantic_datasets`: a dataset's semantics
//! decide the rows every measure above it aggregates, so a change to them
//! invalidates cached work exactly as a measure change does.

use sea_orm_migration::prelude::*;

pub const REQUIRED_CHECKS: &[&str] = &["chk_semantic_datasets_definition_version_positive"];

const ADD_COLUMN: &str = "ALTER TABLE semantic_datasets \
     ADD COLUMN IF NOT EXISTS definition_version INT NOT NULL DEFAULT 1 AFTER custom_schema";

const ADD_CHECK: &str = "ALTER TABLE semantic_datasets \
     ADD CONSTRAINT chk_semantic_datasets_definition_version_positive \
     CHECK (definition_version >= 1)";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        conn.execute_unprepared(ADD_COLUMN).await?;
        // WORKAROUND: MariaDB has no `ADD CONSTRAINT IF NOT EXISTS`, so a
        // duplicate constraint is the success case on a re-run.
        if let Err(error) = conn.execute_unprepared(ADD_CHECK).await {
            let message = error.to_string();
            if !message.contains("Duplicate") && !message.contains("already exists") {
                return Err(error);
            }
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datasets_start_at_the_same_version_as_every_other_definition() {
        assert!(ADD_COLUMN.contains("NOT NULL DEFAULT 1"));
    }

    #[test]
    fn the_check_this_migration_adds_is_the_one_the_probe_requires() {
        for check in REQUIRED_CHECKS {
            assert!(ADD_CHECK.contains(check), "missing CHECK {check}");
        }
    }
}
