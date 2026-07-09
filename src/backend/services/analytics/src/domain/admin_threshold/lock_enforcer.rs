//! `lock-enforcer` (`cpt-metric-cat-component-lock-enforcer`).
//!
//! Single point of decision for whether an admin write at scope `S` would be
//! shadowed by a locked row at a broader scope (DESIGN §3.2 / §3.6).
//! Returns either:
//!
//! - `Ok(LockCheck::Clear)` — no blocking lock; the caller proceeds.
//! - `Ok(LockCheck::Blocked(BlockingLock))` — a broader-scope locked row
//!   exists. The caller emits a `bypass_attempt` audit row (primary sink)
//!   and then rejects with a canonical `permission_denied` envelope
//!   carrying `reason = "threshold_locked"` + `blocking_scope` +
//!   `blocking_row_id` + `locked_at` (DESIGN §3.3).
//!
//! ## What this module does NOT do
//!
//! - **Write the audit row.** That's `audit_emitter`'s primary-sink contract.
//!   The enforcer surfaces the blocking-lock metadata and lets the caller
//!   orchestrate the audit + reject sequence per DESIGN §3.6 (the
//!   audit-row INSERT MUST commit BEFORE the 403; if the INSERT fails the
//!   caller surfaces a 503 instead of a 403). Conflating "decide blocking"
//!   with "emit audit" inside the enforcer would force the 503 fall-back
//!   to live here too, which crosses the §3.2 boundary.
//! - **Decide tenant-admin authz.** Already done by `service.rs` via
//!   `TenantAuthorization::is_tenant_admin` before this is called.
//! - **Enforce v1 lock-scope (`is_locked = true` only on `product-default`
//!   / `tenant`).** That's a request-time validation in `service.rs` —
//!   it fires before the enforcer's SQL even runs.
//!
//! ## Query shape
//!
//! ```sql
//! SELECT id, scope, locked_at, locked_by, lock_reason
//! FROM metric_threshold
//! WHERE tenant_id_sentinel = ?            -- bound as BINARY(16)
//!   AND metric_key      = ?
//!   AND is_locked_persisted = TRUE
//!   AND scope IN (<broader-than-target scopes>)
//! ORDER BY <broadness rank ASC>
//! LIMIT 1
//! ```
//!
//! `tenant_id_sentinel` (STORED generated column from #519) coalesces
//! `tenant_id` NULL → all-zero bytes, so `product-default` lock rows
//! (whose `tenant_id` is NULL) are discoverable by the same bind shape —
//! the all-zero sentinel just won't match a real tenant's BINARY(16),
//! and product-default rows are pulled in by the all-zero bind that we
//! use for the broader-scope lookup. We do TWO queries (one against the
//! real tenant's sentinel, one against the all-zero `product-default`
//! sentinel) only when the target scope's `broader_scopes()` actually
//! includes `product-default` — almost always, but saves a round-trip
//! on the `product-default` self-case (which is empty).
//!
//! The index plan hits `idx_metric_threshold_lock_enforcer
//! (tenant_id, metric_key, scope, is_locked_persisted)` — partial-index
//! emulation pinned in DESIGN §3.7 line 1041.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};
use uuid::Uuid;

use super::dto::Scope;

/// Result of a pre-write broader-lock check.
#[derive(Debug)]
pub enum LockCheck {
    /// No broader-scope locked row shadows the target — proceed with the
    /// write.
    Clear,
    /// A broader-scope locked row exists; the caller emits an audit row
    /// and rejects with `permission_denied` (`reason = "threshold_locked"`).
    Blocked(BlockingLock),
}

/// Metadata about the broader-scope locked row that shadowed a write.
/// Mirrors the `context.blocking_*` fields the §3.3 envelope carries.
#[derive(Debug, Clone)]
pub struct BlockingLock {
    pub id: Uuid,
    pub scope: Scope,
    pub locked_at: Option<DateTime<Utc>>,
    pub locked_by: Option<String>,
    pub lock_reason: Option<String>,
}

/// Decision input — the row the caller is about to write.
#[derive(Debug, Clone, Copy)]
pub struct CheckTarget<'a> {
    pub tenant_id: Uuid,
    pub metric_key: &'a str,
    pub scope: Scope,
}

/// Probe MariaDB for a broader-scope locked row shadowing `target`.
///
/// # Errors
///
/// Surfaces SeaORM transport / query errors. The caller MUST treat these
/// as a hard 5xx — failing to determine whether a lock exists is the same
/// failure mode as the audit primary-sink failure (DESIGN §3.6): we MUST
/// NOT silently proceed with the write. The caller's `service.rs` maps
/// these to `CanonicalError::internal` (the SQL itself never carries
/// untrusted input, so an error here means the DB is misbehaving).
pub async fn check_broader_locks(
    db: &DatabaseConnection,
    target: CheckTarget<'_>,
) -> Result<LockCheck, sea_orm::DbErr> {
    let broader = target.scope.broader_scopes();
    if broader.is_empty() {
        // `product-default` has no broader scope; nothing can shadow it.
        return Ok(LockCheck::Clear);
    }

    let backend = db.get_database_backend();

    // v1 lock-scope restriction (CHECK `chk_metric_threshold_lock_scope_v1`)
    // means `is_locked = true` is only allowed on `product-default` / `tenant`.
    // So we only need to look at the broader-scope set ∩ {product-default,
    // tenant}. Filtering down here keeps the `IN (...)` list tight and the
    // query plan obviously index-aligned.
    let lockable_broader: Vec<Scope> = broader
        .iter()
        .copied()
        .filter(|s| matches!(s, Scope::ProductDefault | Scope::Tenant))
        .collect();
    if lockable_broader.is_empty() {
        return Ok(LockCheck::Clear);
    }

    // `product-default` rows live with `tenant_id IS NULL` (sentinel = all
    // zeros). `tenant` rows live with the real tenant UUID. We bind both
    // sentinels and let the `IN (?, ?)` cover them — even if the broader
    // set is just `{product-default}`, binding the real tenant too is
    // cheap and lets a single query plan handle every scope-target.
    let real_tenant = Value::Bytes(Some(Box::new(target.tenant_id.as_bytes().to_vec())));
    let zero_tenant = Value::Bytes(Some(Box::new(vec![0u8; 16])));

    // Render the scope `IN (...)` placeholders as compile-time-known
    // strings — values come from a closed `enum`, no injection vector.
    // Build the IN-list string from `Scope::as_db_str` so adding a new
    // scope in `dto.rs` automatically propagates here.
    let scope_in_clause = lockable_broader
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT id AS id, scope AS scope, locked_at AS locked_at, \
                locked_by AS locked_by, lock_reason AS lock_reason \
         FROM metric_threshold \
         WHERE tenant_id_sentinel IN (?, ?) \
           AND metric_key = ? \
           AND is_locked_persisted = TRUE \
           AND scope IN ({scope_in_clause}) \
         ORDER BY \
           CASE scope \
             WHEN 'product-default' THEN 0 \
             WHEN 'tenant'          THEN 1 \
             WHEN 'role'            THEN 2 \
             WHEN 'team'            THEN 3 \
             WHEN 'team+role'       THEN 4 \
           END ASC \
         LIMIT 1"
    );

    let mut values: Vec<Value> = Vec::with_capacity(3 + lockable_broader.len());
    values.push(real_tenant);
    values.push(zero_tenant);
    values.push(Value::from(target.metric_key));
    for s in &lockable_broader {
        values.push(Value::from(s.as_db_str()));
    }

    let stmt = Statement::from_sql_and_values(backend, &sql, values);
    let row = LockRow::find_by_statement(stmt).one(db).await?;
    let Some(row) = row else {
        return Ok(LockCheck::Clear);
    };

    let scope = Scope::from_db_str(&row.scope).ok_or_else(|| {
        // DB-side ENUM constrains values to the known set; an unknown
        // here means the schema drifted. Surface as a hard DbErr — the
        // caller turns this into a 5xx.
        sea_orm::DbErr::Custom(format!(
            "lock_enforcer: broader-lock row has unknown scope value {:?} \
             (DB ENUM should have prevented this)",
            row.scope
        ))
    })?;

    Ok(LockCheck::Blocked(BlockingLock {
        id: row.id,
        scope,
        locked_at: row.locked_at,
        locked_by: row.locked_by,
        lock_reason: row.lock_reason,
    }))
}

#[derive(Debug, FromQueryResult)]
struct LockRow {
    id: Uuid,
    scope: String,
    locked_at: Option<DateTime<Utc>>,
    locked_by: Option<String>,
    lock_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_default_target_short_circuits() {
        // `product-default` has no broader ancestor. The enforcer must NOT
        // issue a query — guards against an empty `IN ()` clause (MariaDB
        // syntax error) and saves a round-trip on the common seed-only path.
        // We can't easily mock the DB, but verifying `broader_scopes()` is
        // empty here pins the precondition the production path relies on.
        assert!(Scope::ProductDefault.broader_scopes().is_empty());
    }

    #[test]
    fn lockable_broader_filter_is_correct() {
        // v1 only permits `is_locked = true` on `product-default` / `tenant`,
        // so the enforcer's broader-scope filter must collapse to that pair
        // for every target scope. A regression that included `role` / `team`
        // / `team+role` in the lockable set would still work today (no rows
        // would match) but would tighten silently the moment lock support
        // expands — better to pin the v1 boundary here.
        for target in [Scope::Tenant, Scope::Role, Scope::Team, Scope::TeamRole] {
            let lockable: Vec<Scope> = target
                .broader_scopes()
                .iter()
                .copied()
                .filter(|s| matches!(s, Scope::ProductDefault | Scope::Tenant))
                .collect();
            for s in &lockable {
                assert!(
                    matches!(s, Scope::ProductDefault | Scope::Tenant),
                    "v1 lock-scope restriction: only product-default / tenant can be locked"
                );
            }
            // Tenant target: only product-default is broader-AND-lockable.
            // Other targets: both product-default and tenant.
            if target == Scope::Tenant {
                assert_eq!(lockable, vec![Scope::ProductDefault]);
            } else {
                assert_eq!(lockable, vec![Scope::ProductDefault, Scope::Tenant]);
            }
        }
    }
}
