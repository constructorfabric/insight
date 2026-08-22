//! `feedback` — what a person told us from inside the product.
//!
//! Authored content, not an observation, so it lives in the service database
//! beside `saved_queries` rather than in ClickHouse beside the usage events:
//! a submission is one row written while its author waits, and a triage state
//! on top of it is an UPDATE.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Feedback::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Feedback::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Feedback::InsightTenantId).uuid().not_null())
                    .col(ColumnDef::new(Feedback::PersonId).uuid().not_null())
                    .col(ColumnDef::new(Feedback::Message).text().not_null())
                    .col(ColumnDef::new(Feedback::Path).string_len(512).not_null())
                    .col(ColumnDef::new(Feedback::AppName).string_len(64).not_null())
                    .col(
                        ColumnDef::new(Feedback::AppVersion)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Feedback::CreatedAt)
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
                    .name("idx_feedback_tenant_created")
                    .table(Feedback::Table)
                    .col(Feedback::InsightTenantId)
                    .col(Feedback::CreatedAt)
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
enum Feedback {
    Table,
    Id,
    InsightTenantId,
    PersonId,
    Message,
    Path,
    AppName,
    AppVersion,
    CreatedAt,
}
