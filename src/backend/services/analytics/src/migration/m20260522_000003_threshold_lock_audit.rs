//! Create `threshold_lock_audit` — append-only audit log for lock lifecycle
//! events and rejected bypass attempts.
//!
//! Refs #519. Schema source: `docs/domain/metric-catalog/specs/DESIGN.md` §3.7
//! (`cpt-metric-cat-dbtable-threshold-lock-audit`).
//!
//! Append-only-ness is enforced at the application layer (audit-emitter is
//! the sole writer). Column-shape integrity is enforced by the schema:
//! `event_type` is an inline ENUM, and the two scope columns
//! (`attempted_scope`, `blocking_scope`) are domain-constrained by CHECK
//! constraints registered in [`REQUIRED_CHECKS`] so the startup probe
//! enforces presence. The CHECK names match `chk_threshold_lock_audit_*` —
//! the audit row is the forensic record for lock-bypass attempts; a row
//! whose `attempted_scope` is an arbitrary string would distort
//! `GROUP BY attempted_scope` histograms in retrospective security audits.
//!
//! Retention is operator-managed (canonical policy ≥ 1 year per §3.7 line 1081).
//! Indexes back per-tenant and per-metric audit lookups; no in-app read path
//! is wired in v1.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Names of every CHECK constraint this migration adds. The startup probe
/// asserts each one is present in `INFORMATION_SCHEMA.CHECK_CONSTRAINTS`.
pub const REQUIRED_CHECKS: &[&str] = &[
    "chk_threshold_lock_audit_attempted_scope",
    "chk_threshold_lock_audit_blocking_scope",
];

/// CHECK clause SQL, ordered to match [`REQUIRED_CHECKS`]. The scope value
/// list is the same set as `metric_threshold.scope` — keep in sync if a new
/// scope is added there.
const CHECK_CLAUSES: &[(&str, &str)] = &[
    (
        "chk_threshold_lock_audit_attempted_scope",
        "attempted_scope IS NULL OR attempted_scope IN \
         ('product-default','tenant','role','team','team+role')",
    ),
    (
        "chk_threshold_lock_audit_blocking_scope",
        "blocking_scope IS NULL OR blocking_scope IN \
         ('product-default','tenant','role','team','team+role')",
    ),
];

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

        let conn = manager.get_connection();
        // `{name}` is backtick-quoted defensively. Both `name` and `predicate`
        // come from the compile-time `CHECK_CLAUSES` const above — no
        // injection vector today — but the backticks make a future entry
        // whose name happens to be a MariaDB reserved word still parse.
        for (name, predicate) in CHECK_CLAUSES {
            conn.execute_unprepared(&format!(
                "ALTER TABLE threshold_lock_audit ADD CONSTRAINT `{name}` CHECK ({predicate})"
            ))
            .await?;
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn predicate_for(name: &str) -> &'static str {
        let Some((_, p)) = CHECK_CLAUSES.iter().find(|(n, _)| *n == name) else {
            panic!("CHECK {name} not registered in CHECK_CLAUSES");
        };
        p
    }

    #[test]
    fn check_clauses_match_required_list() {
        let clause_names: Vec<&str> = CHECK_CLAUSES.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            clause_names.as_slice(),
            REQUIRED_CHECKS,
            "CHECK_CLAUSES names must equal REQUIRED_CHECKS in the same order — \
             the startup probe relies on this list to detect drops"
        );
    }

    /// The audit table's scope columns must list every scope value that
    /// `metric_threshold.scope` accepts. Drifting these lists would silently
    /// degrade the forensic audit log: a `bypass_attempt` row recording a
    /// scope value the audit CHECK rejects would fail the INSERT, dropping
    /// the audit signal exactly when it's most needed. Keep in sync with
    /// the inline ENUM in `m20260522_000002_metric_threshold` (scope column).
    #[test]
    fn attempted_scope_check_lists_every_canonical_scope() {
        let p = predicate_for("chk_threshold_lock_audit_attempted_scope");
        for v in [
            "'product-default'",
            "'tenant'",
            "'role'",
            "'team'",
            "'team+role'",
        ] {
            assert!(
                p.contains(v),
                "attempted_scope CHECK missing scope value {v}"
            );
        }
        assert!(
            p.contains("IS NULL"),
            "attempted_scope is nullable — CHECK must permit NULL"
        );
    }

    #[test]
    fn blocking_scope_check_lists_every_canonical_scope() {
        let p = predicate_for("chk_threshold_lock_audit_blocking_scope");
        for v in [
            "'product-default'",
            "'tenant'",
            "'role'",
            "'team'",
            "'team+role'",
        ] {
            assert!(
                p.contains(v),
                "blocking_scope CHECK missing scope value {v}"
            );
        }
        assert!(
            p.contains("IS NULL"),
            "blocking_scope is nullable — CHECK must permit NULL"
        );
    }
}
