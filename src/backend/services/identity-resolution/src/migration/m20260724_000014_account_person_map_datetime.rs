//! The FIRST Rust-authored migration — deliberately NOT in the frozen .NET
//! `DbUp` set (one applier for new DDL; see the module docs in
//! `migration/mod.rs`). Converts `account_person_map`'s SCD2 columns to
//! `DATETIME(6)`, aligning with 009's `persons`/`org_chart` conversion.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::apply_sql(
            manager,
            include_str!("sql/014_account_person_map_datetime.sql"),
        )
        .await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(super::irreversible())
    }
}
