//! `saved_queries` — the `presentation.queries` entity (#1965).
//!
//! A saved query is a single `SELECT`/`WITH` over the read-only contract,
//! authored by an analyst and tenant-scoped. It is metadata in the service
//! database (MariaDB), mirroring the `metrics` entity — CRUD never touches
//! ClickHouse; only `/run` does, executing the stored SQL as `presentation_ro`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SavedQueries::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SavedQueries::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SavedQueries::InsightTenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SavedQueries::Name)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(ColumnDef::new(SavedQueries::Description).text())
                    .col(ColumnDef::new(SavedQueries::Sql).text().not_null())
                    .col(
                        ColumnDef::new(SavedQueries::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(SavedQueries::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_saved_queries_tenant")
                    .table(SavedQueries::Table)
                    .col(SavedQueries::InsightTenantId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[derive(DeriveIden)]
enum SavedQueries {
    Table,
    Id,
    InsightTenantId,
    Name,
    Description,
    Sql,
    CreatedAt,
    UpdatedAt,
}
