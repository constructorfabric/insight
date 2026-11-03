use insight_migration::squash::{self, History};
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, DbBackend, Statement};

/// The last releases that still carry the unsquashed history.
const UPGRADE_THROUGH: &str = "insight umbrella 0.7.169, or a release-2026.09 build from 0.6.92 on";

/// Identity tables only the unsquashed history leaves behind: the retired
/// applier's ledger, the binding cache 015 dropped, and `persons` before 009.
const UNLEDGERED_SCHEMA_PROBE: &str = "
    SELECT TABLE_NAME FROM information_schema.TABLES
    WHERE TABLE_SCHEMA = DATABASE()
      AND LOWER(TABLE_NAME) IN ('schemaversions', 'account_person_map')
    UNION ALL
    SELECT TABLE_NAME FROM information_schema.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE()
      AND TABLE_NAME = 'persons' AND COLUMN_NAME = 'created_at' AND DATA_TYPE = 'timestamp'
";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        match squash::history::<super::Migrator>(manager, super::SQUASHED, UPGRADE_THROUGH).await? {
            History::Complete => Ok(()),
            History::Fresh => {
                refuse_unledgered_schema(manager).await?;
                super::apply_sql(manager, include_str!("sql/000_baseline.sql")).await
            }
        }
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(super::irreversible())
    }
}

/// Refuse an empty ledger over a schema the unsquashed history built.
///
/// The baseline only creates what is missing, so over such a schema it would
/// record the history as walked while leaving the tables in their old shape.
async fn refuse_unledgered_schema(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let rows = manager
        .get_connection()
        .query_all_raw(Statement::from_string(
            DbBackend::MySql,
            UNLEDGERED_SCHEMA_PROBE,
        ))
        .await?;
    let found = rows
        .iter()
        .map(|row| row.try_get_by_index::<String>(0))
        .collect::<Result<Vec<_>, _>>()?;
    if found.is_empty() {
        return Ok(());
    }

    Err(DbErr::Custom(format!(
        "the migration ledger is empty, but the database holds identity tables from before \
         the squashed history ({}). This build cannot bring them forward. First upgrade \
         through {UPGRADE_THROUGH}, then deploy this release.",
        found.join(", "),
    )))
}
