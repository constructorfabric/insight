//! Ported verbatim from the .NET `DbUp` script `008_person_roles.sql` — the SQL file is
//! a byte-for-byte copy; see the module docs in `migration/mod.rs`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::apply_sql(manager, include_str!("sql/008_person_roles.sql")).await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(super::irreversible())
    }
}
