//! Create `threshold_lock_audit` — append-only audit log for lock lifecycle
//! events and rejected bypass attempts.
//!
//! Refs #519. Schema source: `docs/domain/metric-catalog/specs/DESIGN.md` §3.7
//! (`cpt-metric-cat-dbtable-threshold-lock-audit`).
//!
//! No CHECK constraints in v1 per §3.7: append-only-ness is enforced at the
//! application layer (audit-emitter is the sole writer), and the ENUM /
//! NOT-NULL column constraints handle the rest.
//!
//! Retention is operator-managed (canonical policy ≥ 1 year per §3.7 line 1081).
//! Indexes back per-tenant and per-metric audit lookups; no in-app read path
//! is wired in v1.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// No CHECK constraints on this table in v1 — the audit row's invariants are
/// either column-type (ENUMs, NOT NULL) or application-layer (append-only).
pub const REQUIRED_CHECKS: &[&str] = &[];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    #[allow(clippy::too_many_lines)] // single-table DDL — splitting hurts readability
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ThresholdLockAudit::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ThresholdLockAudit::Id)
                            .binary_len(16)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::EventType)
                            .custom(Alias::new(
                                "ENUM('bypass_attempt','lock_set','lock_cleared')",
                            ))
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::ActorSubject)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::TenantId)
                            .binary_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::MetricKey)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::AttemptedScope)
                            .string_len(32)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::AttemptedValues)
                            .json()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::BlockingScope)
                            .string_len(32)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::BlockingRowId)
                            .binary_len(16)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::LockedBy)
                            .string_len(128)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::LockedAt)
                            .timestamp()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::LockReason)
                            .string_len(512)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::EventAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ThresholdLockAudit::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_threshold_lock_audit_tenant_time")
                    .table(ThresholdLockAudit::Table)
                    .col(ThresholdLockAudit::TenantId)
                    .col(ThresholdLockAudit::EventAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_threshold_lock_audit_metric_time")
                    .table(ThresholdLockAudit::Table)
                    .col(ThresholdLockAudit::MetricKey)
                    .col(ThresholdLockAudit::EventAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // we have only forward migrations
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[derive(DeriveIden)]
enum ThresholdLockAudit {
    Table,
    Id,
    EventType,
    ActorSubject,
    TenantId,
    MetricKey,
    AttemptedScope,
    AttemptedValues,
    BlockingScope,
    BlockingRowId,
    LockedBy,
    LockedAt,
    LockReason,
    EventAt,
    CreatedAt,
}
