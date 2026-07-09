//! Create `metric_threshold` — per-scope thresholds with v1 lock-bounded escalation.
//!
//! Refs #519. Schema source: `docs/domain/metric-catalog/specs/DESIGN.md` §3.7
//! (`cpt-metric-cat-dbtable-metric-threshold`).
//!
//! Notes that drive non-obvious choices:
//!
//! - `role_slug` / `team_id` are `NOT NULL DEFAULT ''` (empty-string sentinel),
//!   not nullable. SQL treats NULLs as distinct, which would let duplicate
//!   `product-default` rows past the UNIQUE composite — sentinels make the
//!   composite actually unique. See DESIGN §3.7 lines 1011-1012, 1029.
//! - `tenant_id` is the same story: NULL-tenant `product-default` rows would
//!   otherwise duplicate under the UNIQUE composite. We can't make `tenant_id`
//!   itself NOT NULL (DESIGN intentionally distinguishes NULL = "product
//!   default" from "<all-zero> = some tenant"). Mirror it through a STORED
//!   generated `tenant_id_sentinel` column (all-zero bytes when NULL) and use
//!   that in the UNIQUE composite. Same mechanism `is_locked_persisted` uses
//!   for partial-index emulation.
//! - `is_locked_persisted` is a STORED generated mirror of `is_locked`. MariaDB
//!   has no native partial indexes; the lock-enforcer's "find broader-scope
//!   locked row" lookup uses `(tenant_id, metric_key, scope, is_locked_persisted)`
//!   as the supported workaround. See DESIGN §3.7 lines 1021, 1041.
//! - **No `FOREIGN KEY (metric_key) REFERENCES metric_catalog(metric_key)`**
//!   in v1. This is deliberate per DESIGN §3.7 line 309 ("Metric 1 — N
//!   Threshold by `metric_key`. No FK in v1 ...; CRUD validates that
//!   `metric_key` exists and is `is_enabled = true` before allowing a write
//!   per `cpt-metric-cat-fr-threshold-crud`"). The audit row's "loose
//!   pointer" treatment in §3.7 lines 310-311 explicitly relies on
//!   audit-survives-deletion-of-parent semantics; an FK would break that
//!   contract. If a future revision tightens this, amend §3.7 first via
//!   ADR — adding the FK silently is the wrong move.
//! - CHECK names below match `REQUIRED_CHECKS` and the startup probe.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Names of every CHECK constraint this migration adds. The startup probe
/// asserts each one is present in `INFORMATION_SCHEMA.CHECK_CONSTRAINTS`.
pub const REQUIRED_CHECKS: &[&str] = &[
    "chk_metric_threshold_lock_reason_when_locked",
    "chk_metric_threshold_lock_scope_v1",
    "chk_metric_threshold_lock_reason_length",
    "chk_metric_threshold_role_slug_shape",
    "chk_metric_threshold_team_id_shape",
];

/// CHECK clause SQL, ordered to match [`REQUIRED_CHECKS`].
const CHECK_CLAUSES: &[(&str, &str)] = &[
    (
        "chk_metric_threshold_lock_reason_when_locked",
        "is_locked = FALSE OR lock_reason IS NOT NULL",
    ),
    (
        "chk_metric_threshold_lock_scope_v1",
        "is_locked = FALSE OR scope IN ('product-default','tenant')",
    ),
    (
        "chk_metric_threshold_lock_reason_length",
        "lock_reason IS NULL OR CHAR_LENGTH(lock_reason) <= 512",
    ),
    (
        "chk_metric_threshold_role_slug_shape",
        "(scope IN ('role','team+role') AND role_slug <> '') \
         OR (scope NOT IN ('role','team+role') AND role_slug = '')",
    ),
    (
        "chk_metric_threshold_team_id_shape",
        "(scope IN ('team','team+role') AND team_id <> '') \
         OR (scope NOT IN ('team','team+role') AND team_id = '')",
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    #[allow(clippy::too_many_lines)] // single-table DDL — splitting hurts readability
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MetricThreshold::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MetricThreshold::Id)
                            .binary_len(16)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::TenantId)
                            .binary_len(16)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::MetricKey)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::Scope)
                            .custom(Alias::new(
                                "ENUM('product-default','tenant','role','team','team+role')",
                            ))
                            .not_null(),
                    )
                    // Empty-string sentinel — NOT NULL DEFAULT '' so the UNIQUE
                    // composite on (tenant_id, metric_key, scope, role_slug, team_id)
                    // doesn't degrade to "NULLs are distinct".
                    .col(
                        ColumnDef::new(MetricThreshold::RoleSlug)
                            .string_len(64)
                            .not_null()
                            .default(""),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::TeamId)
                            .string_len(64)
                            .not_null()
                            .default(""),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::Good)
                            .decimal_len(20, 6)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::Warn)
                            .decimal_len(20, 6)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::AlertTrigger)
                            .decimal_len(20, 6)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::AlertBad)
                            .decimal_len(20, 6)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::IsLocked)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::LockedBy)
                            .string_len(128)
                            .null(),
                    )
                    .col(ColumnDef::new(MetricThreshold::LockedAt).timestamp().null())
                    .col(
                        ColumnDef::new(MetricThreshold::LockReason)
                            .string_len(512)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(MetricThreshold::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp())
                            .extra("ON UPDATE CURRENT_TIMESTAMP"),
                    )
                    .to_owned(),
            )
            .await?;

        let conn = manager.get_connection();

        // Generated columns added separately — sea-orm 1.1's column builder doesn't
        // emit MariaDB's `GENERATED ALWAYS AS (...) STORED` cleanly with the
        // NOT-NULL position we need. Raw SQL keeps the produced DDL unambiguous.
        //
        // `tenant_id_sentinel`: all-zero 16-byte BINARY when `tenant_id IS NULL`,
        // otherwise mirrors `tenant_id`. Folded into the UNIQUE composite below
        // so NULL-tenant `product-default` rows actually deduplicate (SQL's
        // NULL-is-distinct semantics on raw `tenant_id` would let dupes
        // through). The `0x` literal is 16 zero bytes — same width as
        // `BINARY(16)`, no padding ambiguity. UUIDv7 has no all-zero generator
        // bits so this sentinel cannot collide with a real tenant id.
        conn.execute_unprepared(
            "ALTER TABLE metric_threshold \
             ADD COLUMN tenant_id_sentinel BINARY(16) \
             GENERATED ALWAYS AS (COALESCE(tenant_id, 0x00000000000000000000000000000000)) \
             STORED",
        )
        .await?;

        conn.execute_unprepared(
            "ALTER TABLE metric_threshold \
             ADD COLUMN is_locked_persisted BOOLEAN \
             GENERATED ALWAYS AS (is_locked) STORED",
        )
        .await?;

        // UNIQUE composite doubles as the resolver lookup index (§3.7 line 1040).
        // Built via raw SQL so we can lead with `tenant_id_sentinel` (a generated
        // column not declared in the SeaORM `Iden` enum) instead of the raw
        // nullable `tenant_id` — the latter would let two NULL-tenant
        // product-default rows past the constraint.
        conn.execute_unprepared(
            "CREATE UNIQUE INDEX uq_metric_threshold_scope_target \
             ON metric_threshold \
             (tenant_id_sentinel, metric_key, scope, role_slug, team_id)",
        )
        .await?;

        // Lock-enforcer hot-path index (partial-index emulation via the generated
        // column — §3.7 line 1041). Built via raw SQL so we can reference the
        // generated column without re-declaring it in the SeaORM `Iden` enum.
        conn.execute_unprepared(
            "CREATE INDEX idx_metric_threshold_lock_enforcer \
             ON metric_threshold (tenant_id, metric_key, scope, is_locked_persisted)",
        )
        .await?;

        // `{name}` is backtick-quoted defensively. Both `name` and `predicate`
        // come from the compile-time `CHECK_CLAUSES` const above — no
        // injection vector today — but the backticks make a future entry
        // whose name happens to be a MariaDB reserved word still parse.
        for (name, predicate) in CHECK_CLAUSES {
            conn.execute_unprepared(&format!(
                "ALTER TABLE metric_threshold ADD CONSTRAINT `{name}` CHECK ({predicate})"
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
enum MetricThreshold {
    Table,
    Id,
    TenantId,
    MetricKey,
    Scope,
    RoleSlug,
    TeamId,
    Good,
    Warn,
    AlertTrigger,
    AlertBad,
    IsLocked,
    LockedBy,
    LockedAt,
    LockReason,
    // Generated columns added via raw SQL after table creation (see the
    // ALTER TABLE block above). Listed here so they show up in grep / IDE
    // refactors even though no in-crate code currently references them
    // through SeaORM.
    #[allow(dead_code)]
    TenantIdSentinel,
    #[allow(dead_code)]
    IsLockedPersisted,
    CreatedAt,
    UpdatedAt,
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

    #[test]
    fn lock_reason_when_locked_predicate_is_correct() {
        let p = predicate_for("chk_metric_threshold_lock_reason_when_locked");
        assert!(p.contains("is_locked = FALSE"));
        assert!(p.contains("lock_reason IS NOT NULL"));
    }

    #[test]
    fn v1_lock_scope_restricted_to_product_default_and_tenant() {
        let p = predicate_for("chk_metric_threshold_lock_scope_v1");
        assert!(p.contains("'product-default'"));
        assert!(p.contains("'tenant'"));
        // v1: role / team / team+role MUST NOT appear here. Adding them would
        // permit the lock-escalation path admin-crud is meant to block.
        assert!(!p.contains("'role'"));
        assert!(!p.contains("'team'"));
        assert!(!p.contains("'team+role'"));
    }

    #[test]
    fn role_slug_shape_predicate_uses_sentinel_logic() {
        let p = predicate_for("chk_metric_threshold_role_slug_shape");
        assert!(p.contains("role_slug <> ''"));
        assert!(p.contains("role_slug = ''"));
        // Must enumerate role-bearing scopes.
        assert!(p.contains("'role'"));
        assert!(p.contains("'team+role'"));
    }

    #[test]
    fn team_id_shape_predicate_uses_sentinel_logic() {
        let p = predicate_for("chk_metric_threshold_team_id_shape");
        assert!(p.contains("team_id <> ''"));
        assert!(p.contains("team_id = ''"));
        assert!(p.contains("'team'"));
        assert!(p.contains("'team+role'"));
    }
}
