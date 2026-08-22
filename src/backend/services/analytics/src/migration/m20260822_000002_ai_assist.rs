//! Tables behind "explain this metric with AI".
//!
//! Three authored things (a tenant's system prompt, its context entries, a
//! person's own entries) and one secret (a person's Anthropic token, sealed).
//! All service-database rows: authored content, never observations.

use sea_orm_migration::prelude::*;

/// The column budgets, read by the validators that clip to them.
pub mod ai_assist_schema {
    pub const TITLE: u32 = 255;

    /// `body` and `system_prompt` are TEXT; this is the product's own limit.
    pub const BODY: usize = 8000;

    /// Ciphertext of one Anthropic token plus its GCM tag, with headroom.
    pub const CIPHERTEXT: u32 = 512;

    pub const NONCE: u32 = 12;

    pub const HINT: u32 = 4;

    /// How many entries one scope may hold.
    pub const MAX_ENTRIES_PER_SCOPE: u64 = 20;

    /// How many characters of context may reach one prompt.
    pub const MAX_CONTEXT_CHARS: usize = 20_000;
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    #[expect(
        clippy::too_many_lines,
        reason = "three table definitions read better together than split across helpers"
    )]
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AiContextEntries::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AiContextEntries::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AiContextEntries::InsightTenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiContextEntries::Scope)
                            .string_len(16)
                            .not_null(),
                    )
                    .col(ColumnDef::new(AiContextEntries::PersonId).uuid())
                    .col(
                        ColumnDef::new(AiContextEntries::Title)
                            .string_len(ai_assist_schema::TITLE)
                            .not_null(),
                    )
                    .col(ColumnDef::new(AiContextEntries::Body).text().not_null())
                    .col(
                        ColumnDef::new(AiContextEntries::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(AiContextEntries::UpdatedAt)
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
                    .name("idx_ai_context_entries_scope")
                    .table(AiContextEntries::Table)
                    .col(AiContextEntries::InsightTenantId)
                    .col(AiContextEntries::Scope)
                    .col(AiContextEntries::PersonId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AiCredentials::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AiCredentials::InsightTenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AiCredentials::PersonId).uuid().not_null())
                    .col(
                        ColumnDef::new(AiCredentials::Nonce)
                            .var_binary(ai_assist_schema::NONCE)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiCredentials::Ciphertext)
                            .var_binary(ai_assist_schema::CIPHERTEXT)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiCredentials::Hint)
                            .string_len(ai_assist_schema::HINT)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiCredentials::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(AiCredentials::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .primary_key(
                        Index::create()
                            .col(AiCredentials::InsightTenantId)
                            .col(AiCredentials::PersonId),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AiSettings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AiSettings::InsightTenantId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AiSettings::SystemPrompt).text())
                    .col(
                        ColumnDef::new(AiSettings::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
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
enum AiContextEntries {
    Table,
    Id,
    InsightTenantId,
    Scope,
    PersonId,
    Title,
    Body,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum AiCredentials {
    Table,
    InsightTenantId,
    PersonId,
    Nonce,
    Ciphertext,
    Hint,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum AiSettings {
    Table,
    InsightTenantId,
    SystemPrompt,
    UpdatedAt,
}
