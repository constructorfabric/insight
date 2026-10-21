//! Dashboards, folders and tags gain an owner; dashboards gain the people
//! they are shared with and each person's own filing of them.
//!
//! Every row already stored stays ownerless. Folder names stop being unique,
//! and a tag name is unique within its owner's scope rather than everywhere.
//!
//! INVARIANT: every step is safe to run again. Columns, indexes and tables
//! are added with `IF NOT EXISTS` and keys dropped with `IF EXISTS`, and the
//! scoped tag key is in place before the global one goes, so a run stopped
//! at any step and run again ends where one whole run does.

use sea_orm::DbErr;
use sea_orm_migration::prelude::*;

const STEPS: [&str; 11] = [
    "ALTER TABLE dashboards ADD COLUMN IF NOT EXISTS owner_person_id CHAR(36) NULL",
    "CREATE INDEX IF NOT EXISTS dashboards_owner ON dashboards (owner_person_id)",
    "ALTER TABLE folders ADD COLUMN IF NOT EXISTS owner_person_id CHAR(36) NULL",
    "CREATE INDEX IF NOT EXISTS folders_owner ON folders (owner_person_id)",
    "DROP INDEX IF EXISTS folders_name ON folders",
    "ALTER TABLE tags ADD COLUMN IF NOT EXISTS owner_person_id CHAR(36) NULL",
    "ALTER TABLE tags ADD COLUMN IF NOT EXISTS owner_scope CHAR(36) AS (COALESCE(owner_person_id, '')) STORED",
    "CREATE UNIQUE INDEX IF NOT EXISTS tags_owner_name ON tags (owner_scope, name)",
    "DROP INDEX IF EXISTS tags_name ON tags",
    "CREATE TABLE IF NOT EXISTS dashboard_shares (
    dashboard VARCHAR(128) NOT NULL,
    person CHAR(36) NOT NULL,
    granted_by CHAR(36) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    PRIMARY KEY (dashboard, person),
    KEY dashboard_shares_person (person, dashboard),
    CONSTRAINT dashboard_shares_dashboard FOREIGN KEY (dashboard)
        REFERENCES dashboards (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
    "CREATE TABLE IF NOT EXISTS dashboard_placements (
    person CHAR(36) NOT NULL,
    dashboard VARCHAR(128) NOT NULL,
    folder_id CHAR(36) NOT NULL,
    PRIMARY KEY (person, dashboard),
    KEY dashboard_placements_dashboard (dashboard),
    KEY dashboard_placements_folder (folder_id),
    CONSTRAINT dashboard_placements_dashboard FOREIGN KEY (dashboard)
        REFERENCES dashboards (id) ON DELETE CASCADE,
    CONSTRAINT dashboard_placements_folder FOREIGN KEY (folder_id)
        REFERENCES folders (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
];

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261021_000010_dashboard_ownership"
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
            "dropping ownership would show every private dashboard to everyone".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::STEPS;

    fn step(part: &str) -> usize {
        STEPS
            .iter()
            .position(|step| step.contains(part))
            .unwrap_or_else(|| panic!("no step holds {part:?}"))
    }

    #[test]
    fn every_step_is_one_that_runs_again_without_change() {
        for statement in STEPS {
            let guarded = statement.contains("IF NOT EXISTS") || statement.contains("IF EXISTS");
            assert!(guarded, "should be guarded: {statement}");
        }
    }

    #[test]
    fn a_tag_name_stays_unique_within_its_scope_through_the_key_swap() {
        assert!(
            step("CREATE UNIQUE INDEX IF NOT EXISTS tags_owner_name ON tags (owner_scope, name)")
                < step("DROP INDEX IF EXISTS tags_name"),
            "the scoped key must be in place before the global one goes"
        );
        assert!(
            step("owner_scope CHAR(36) AS (COALESCE(owner_person_id, '')) STORED")
                < step("tags_owner_name"),
            "the key reads the generated scope"
        );
    }

    #[test]
    fn existing_rows_stay_ownerless() {
        for table in ["dashboards", "folders", "tags"] {
            let added = format!(
                "ALTER TABLE {table} ADD COLUMN IF NOT EXISTS owner_person_id CHAR(36) NULL"
            );
            assert!(STEPS.contains(&added.as_str()), "{added}");
        }
        assert!(
            STEPS.iter().all(|step| !step.starts_with("UPDATE")),
            "no step assigns an owner"
        );
    }

    #[test]
    fn shares_and_placements_go_with_their_dashboard_and_folder() {
        let shares = STEPS[step("CREATE TABLE IF NOT EXISTS dashboard_shares")];
        let placements = STEPS[step("CREATE TABLE IF NOT EXISTS dashboard_placements")];

        for part in [
            "PRIMARY KEY (dashboard, person)",
            "KEY dashboard_shares_person (person, dashboard)",
            "REFERENCES dashboards (id) ON DELETE CASCADE",
        ] {
            assert!(shares.contains(part), "{part} is missing: {shares}");
        }
        for part in [
            "PRIMARY KEY (person, dashboard)",
            "REFERENCES dashboards (id) ON DELETE CASCADE",
            "REFERENCES folders (id) ON DELETE CASCADE",
        ] {
            assert!(placements.contains(part), "{part} is missing: {placements}");
        }
    }
}
