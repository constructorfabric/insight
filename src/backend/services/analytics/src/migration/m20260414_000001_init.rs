//! Initial schema: `metrics`, `thresholds`, `table_columns`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    #[allow(clippy::too_many_lines)] // migration DDL — splitting would reduce readability
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ── metrics ─────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Metrics::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Metrics::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Metrics::InsightTenantId).uuid().not_null())
                    .col(ColumnDef::new(Metrics::Name).string_len(255).not_null())
                    .col(ColumnDef::new(Metrics::Description).text())
                    .col(ColumnDef::new(Metrics::QueryRef).text().not_null())
                    .col(
                        ColumnDef::new(Metrics::IsEnabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Metrics::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Metrics::UpdatedAt)
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
                    .name("idx_metrics_tenant_enabled")
                    .table(Metrics::Table)
                    .col(Metrics::InsightTenantId)
                    .col(Metrics::IsEnabled)
                    .to_owned(),
            )
            .await?;

        // ── thresholds ──────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Thresholds::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Thresholds::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Thresholds::InsightTenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Thresholds::MetricId).uuid().not_null())
                    .col(
                        ColumnDef::new(Thresholds::FieldName)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Thresholds::Operator)
                            .string_len(10)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Thresholds::Value)
                            .decimal_len(20, 6)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Thresholds::Level).string_len(20).not_null())
                    .col(
                        ColumnDef::new(Thresholds::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Thresholds::UpdatedAt)
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
                    .name("idx_thresholds_metric_id")
                    .table(Thresholds::Table)
                    .col(Thresholds::MetricId)
                    .to_owned(),
            )
            .await?;

        // ── table_columns ───────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(TableColumns::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TableColumns::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(TableColumns::InsightTenantId).uuid())
                    .col(
                        ColumnDef::new(TableColumns::ClickhouseTable)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TableColumns::FieldName)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(ColumnDef::new(TableColumns::FieldDescription).text())
                    .col(
                        ColumnDef::new(TableColumns::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(TableColumns::UpdatedAt)
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
                    .name("uq_tenant_table_field")
                    .table(TableColumns::Table)
                    .col(TableColumns::InsightTenantId)
                    .col(TableColumns::ClickhouseTable)
                    .col(TableColumns::FieldName)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(TableColumns::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Thresholds::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Metrics::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Metrics {
    Table,
    Id,
    InsightTenantId,
    Name,
    Description,
    QueryRef,
    IsEnabled,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Thresholds {
    Table,
    Id,
    InsightTenantId,
    MetricId,
    FieldName,
    Operator,
    Value,
    Level,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum TableColumns {
    Table,
    Id,
    InsightTenantId,
    ClickhouseTable,
    FieldName,
    FieldDescription,
    CreatedAt,
    UpdatedAt,
}
