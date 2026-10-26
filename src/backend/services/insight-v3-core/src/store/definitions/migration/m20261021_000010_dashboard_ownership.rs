//! Dashboards, folders and tags gain an owner; dashboards and folders gain
//! grants, and folders hold dashboards through entries, any number each.
//!
//! Every row already stored stays ownerless, and a dashboard's legacy folder
//! becomes a carrying entry of it. Folder names stop being unique, and a tag
//! name is unique within its owner's scope rather than everywhere.
//!
//! INVARIANT: every step is safe to run again. Columns, indexes and tables
//! are added with `IF NOT EXISTS`, keys dropped with `IF EXISTS`, entries
//! copied with `INSERT IGNORE`, and the scoped tag key is in place before the
//! global one goes, so a run stopped at any step and run again ends where one
//! whole run does.

use sea_orm::DbErr;
use sea_orm_migration::prelude::*;

const STEPS: [&str; 13] = [
    "ALTER TABLE dashboards ADD COLUMN IF NOT EXISTS owner_person_id CHAR(36) NULL",
    "CREATE INDEX IF NOT EXISTS dashboards_owner ON dashboards (owner_person_id)",
    "ALTER TABLE folders ADD COLUMN IF NOT EXISTS owner_person_id CHAR(36) NULL",
    "CREATE INDEX IF NOT EXISTS folders_owner ON folders (owner_person_id)",
    "DROP INDEX IF EXISTS folders_name ON folders",
    "ALTER TABLE tags ADD COLUMN IF NOT EXISTS owner_person_id CHAR(36) NULL",
    // WORKAROUND: MariaDB refuses a generated column over a bare CHAR, whose
    // padding depends on sql_mode (error 1901); RTRIM makes it deterministic.
    "ALTER TABLE tags ADD COLUMN IF NOT EXISTS owner_scope CHAR(36) AS (COALESCE(RTRIM(owner_person_id), '')) STORED",
    "CREATE UNIQUE INDEX IF NOT EXISTS tags_owner_name ON tags (owner_scope, name)",
    "DROP INDEX IF EXISTS tags_name ON tags",
    "CREATE TABLE IF NOT EXISTS dashboard_grants (
    dashboard VARCHAR(128) NOT NULL,
    grantee VARCHAR(36) NOT NULL,
    level VARCHAR(8) NOT NULL,
    granted_by CHAR(36) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    PRIMARY KEY (dashboard, grantee),
    KEY dashboard_grants_grantee (grantee, dashboard),
    CONSTRAINT dashboard_grants_dashboard FOREIGN KEY (dashboard)
        REFERENCES dashboards (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
    "CREATE TABLE IF NOT EXISTS folder_grants (
    folder_id CHAR(36) NOT NULL,
    grantee VARCHAR(36) NOT NULL,
    level VARCHAR(8) NOT NULL,
    granted_by CHAR(36) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    PRIMARY KEY (folder_id, grantee),
    KEY folder_grants_grantee (grantee, folder_id),
    CONSTRAINT folder_grants_folder FOREIGN KEY (folder_id)
        REFERENCES folders (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
    "CREATE TABLE IF NOT EXISTS folder_entries (
    folder_id CHAR(36) NOT NULL,
    dashboard VARCHAR(128) NOT NULL,
    placed_by CHAR(36) NULL,
    carries BOOLEAN NOT NULL,
    created_at DATETIME(6) NOT NULL,
    PRIMARY KEY (folder_id, dashboard),
    KEY folder_entries_dashboard (dashboard),
    CONSTRAINT folder_entries_folder FOREIGN KEY (folder_id)
        REFERENCES folders (id) ON DELETE CASCADE,
    CONSTRAINT folder_entries_dashboard FOREIGN KEY (dashboard)
        REFERENCES dashboards (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
    "INSERT IGNORE INTO folder_entries (folder_id, dashboard, placed_by, carries, created_at)
SELECT folder_id, id, NULL, TRUE, UTC_TIMESTAMP(6) FROM dashboards WHERE folder_id IS NOT NULL",
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
            let guarded = statement.contains("IF NOT EXISTS")
                || statement.contains("IF EXISTS")
                || statement.starts_with("INSERT IGNORE");
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
            step("owner_scope CHAR(36) AS (COALESCE(RTRIM(owner_person_id), '')) STORED")
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
    fn grants_and_entries_go_with_their_dashboard_and_folder() {
        let cases = [
            (
                "CREATE TABLE IF NOT EXISTS dashboard_grants",
                vec![
                    "PRIMARY KEY (dashboard, grantee)",
                    "KEY dashboard_grants_grantee (grantee, dashboard)",
                    "REFERENCES dashboards (id) ON DELETE CASCADE",
                ],
            ),
            (
                "CREATE TABLE IF NOT EXISTS folder_grants",
                vec![
                    "PRIMARY KEY (folder_id, grantee)",
                    "KEY folder_grants_grantee (grantee, folder_id)",
                    "REFERENCES folders (id) ON DELETE CASCADE",
                ],
            ),
            (
                "CREATE TABLE IF NOT EXISTS folder_entries",
                vec![
                    "PRIMARY KEY (folder_id, dashboard)",
                    "KEY folder_entries_dashboard (dashboard)",
                    "REFERENCES folders (id) ON DELETE CASCADE",
                    "REFERENCES dashboards (id) ON DELETE CASCADE",
                ],
            ),
        ];

        for (table, parts) in cases {
            let created = STEPS[step(table)];
            for part in parts {
                assert!(created.contains(part), "{part} is missing: {created}");
            }
        }
    }

    #[test]
    fn a_legacy_folder_becomes_a_carrying_entry_once_the_table_exists() {
        let copy = step("INSERT IGNORE INTO folder_entries");

        assert!(step("CREATE TABLE IF NOT EXISTS folder_entries") < copy);
        assert!(STEPS[copy].contains("TRUE"), "{}", STEPS[copy]);
        assert!(
            STEPS[copy].contains("WHERE folder_id IS NOT NULL"),
            "{}",
            STEPS[copy]
        );
    }
}
