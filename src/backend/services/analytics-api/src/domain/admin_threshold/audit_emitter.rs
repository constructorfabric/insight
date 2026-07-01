//! `audit-emitter` (`cpt-metric-cat-component-audit-emitter`).
//!
//! Sole writer of `threshold_lock_audit` rows and the corresponding
//! structured-log lines (DESIGN §3.2). The dual-sink contract per
//! `cpt-metric-cat-principle-lock-audit`:
//!
//! - **Primary sink** is the `threshold_lock_audit` row. It MUST commit
//!   before the caller's response is returned.
//! - **Derived async stream** is the structured-log line. A log-sink
//!   failure is observable (counter + dead-letter via `tracing::error!`)
//!   but does NOT roll back the row commit.
//!
//! Three event kinds, three call sites:
//!
//! | Event          | Caller        | Sink mode                                |
//! |----------------|---------------|------------------------------------------|
//! | `lock_set`     | `service.rs`  | INSERT inside the threshold-write TX     |
//! | `lock_cleared` | `service.rs`  | INSERT inside the threshold-write TX     |
//! | `bypass_attempt` | `service.rs`| OWN short TX before returning the 403    |
//!
//! For `lock_set` / `lock_cleared` the atomic-with-write contract means we
//! take an `&impl ConnectionTrait` so the caller passes either a
//! `DatabaseTransaction` (atomic) or a `DatabaseConnection` (standalone).
//! For `bypass_attempt` we take the connection AND wrap the INSERT in our
//! own short transaction so the row commits independently of the
//! threshold-write TX (which never started — the lock-enforcer rejected
//! it before reaching the write step).
//!
//! ## Append-only
//!
//! No `update` / `delete` paths against `threshold_lock_audit`. The
//! entity's `ActiveModelBehavior` is the default (no custom hooks) and
//! every method in this module performs INSERTs only — verified by a
//! grep test in `mod.rs`.

use chrono::{DateTime, Utc};
use sea_orm::{ActiveValue, ConnectionTrait, DatabaseConnection, EntityTrait, TransactionTrait};
use serde_json::json;
use uuid::Uuid;

use toolkit_security::SecurityContext;
use crate::domain::admin_threshold::dto::Scope;
use crate::domain::admin_threshold::lock_enforcer::BlockingLock;
use crate::domain::auth::TenantAuthorization;
use crate::infra::db::entities::threshold_lock_audit;

/// Canonical event kinds — mirrors the DB-side ENUM declared in
/// `migration/m20260522_000003_threshold_lock_audit.rs` line 67.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    LockSet,
    LockCleared,
    BypassAttempt,
}

impl EventKind {
    fn as_db_str(self) -> &'static str {
        match self {
            Self::LockSet => "lock_set",
            Self::LockCleared => "lock_cleared",
            Self::BypassAttempt => "bypass_attempt",
        }
    }
}

/// Payload for the lock-transition audit row (`lock_set` / `lock_cleared`).
/// Inserted atomically with the threshold write that triggered it.
#[derive(Debug, Clone)]
pub struct LockTransition<'a> {
    pub kind: EventKind,
    pub tenant_id: Uuid,
    pub metric_key: &'a str,
    /// `Some(_)` when the lock was set (carries the `locked_by` /
    /// `locked_at` / `lock_reason` of the row that JUST locked) and
    /// `None` when the lock was cleared.
    pub locked_by: Option<String>,
    pub locked_at: Option<DateTime<Utc>>,
    pub lock_reason: Option<String>,
}

/// Payload for the `bypass_attempt` audit row. Captures everything the
/// caller tried to do so a post-hoc compliance review can reconstruct
/// the attempt without joining tables.
#[derive(Debug, Clone)]
pub struct BypassAttempt<'a> {
    pub tenant_id: Uuid,
    pub metric_key: &'a str,
    pub attempted_scope: Scope,
    /// JSON encoding of the request body's threshold values. Capped at 4
    /// KB by the audit-table CHECK; the emitter does NOT truncate — the
    /// caller's request body has already been size-bounded by the
    /// upstream JSON extractor.
    pub attempted_values: serde_json::Value,
    pub blocking: BlockingLock,
}

/// Returned to the caller when the `bypass_attempt` primary sink fails.
/// The caller MUST translate this to a canonical 503 `service_unavailable`
/// envelope with `reason = "audit_unavailable"` (DESIGN §3.6 lock-bypass
/// sequence — "no silent bypass").
#[derive(Debug)]
pub struct AuditPrimarySinkFailure {
    pub cause: sea_orm::DbErr,
}

/// Audit-emitter component. Holds the `TenantAuthorization` handle so it
/// can populate `actor_subject` from the `SecurityContext` on every emit,
/// keeping that derivation in one place (auditing should never know about
/// `SecurityContext.subject_id` directly — when JWT validation lands, the
/// `actor_subject(ctx)` impl updates and audit rows pick up the new
/// principal shape automatically).
#[derive(Clone)]
pub struct AuditEmitter {
    tenant_auth: std::sync::Arc<dyn TenantAuthorization>,
}

impl AuditEmitter {
    #[must_use]
    pub fn new(tenant_auth: std::sync::Arc<dyn TenantAuthorization>) -> Self {
        Self { tenant_auth }
    }

    /// Emit a `lock_set` / `lock_cleared` row. The caller passes either a
    /// `DatabaseTransaction` (the canonical path — atomic with the
    /// threshold write) or a `DatabaseConnection` (only for tests; the
    /// production path is always transactional).
    ///
    /// # Errors
    ///
    /// Surfaces the SeaORM insert error verbatim so the caller's
    /// outer transaction can roll back. The lock-transition contract is
    /// "the audit row and the threshold write commit atomically or both
    /// fail" — the caller MUST propagate this error and abort the
    /// transaction.
    pub async fn emit_lock_transition_in_tx<C>(
        &self,
        conn: &C,
        ctx: &SecurityContext,
        ev: &LockTransition<'_>,
    ) -> Result<(), sea_orm::DbErr>
    where
        C: ConnectionTrait,
    {
        let actor = self.tenant_auth.actor_subject(ctx);
        let now = Utc::now();
        let model = threshold_lock_audit::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            event_type: ActiveValue::Set(ev.kind.as_db_str().to_owned()),
            actor_subject: ActiveValue::Set(actor.clone()),
            tenant_id: ActiveValue::Set(ev.tenant_id),
            metric_key: ActiveValue::Set(ev.metric_key.to_owned()),
            attempted_scope: ActiveValue::Set(None),
            attempted_values: ActiveValue::Set(None),
            blocking_scope: ActiveValue::Set(None),
            blocking_row_id: ActiveValue::Set(None),
            locked_by: ActiveValue::Set(ev.locked_by.clone()),
            locked_at: ActiveValue::Set(ev.locked_at),
            lock_reason: ActiveValue::Set(ev.lock_reason.clone()),
            event_at: ActiveValue::Set(now),
            created_at: ActiveValue::Set(now),
        };
        threshold_lock_audit::Entity::insert(model)
            .exec(conn)
            .await?;
        // Async log emit. We swallow JoinError / channel-full failures
        // here — DESIGN §3.2's "log-sink failure does NOT roll back the
        // row commit" contract is enforced by `tokio::spawn`'s detached
        // task semantics. `actor` is moved into the spawn so we don't
        // re-invoke the trait method (the doc-comment's
        // "auditing should never know about `SecurityContext.subject_id`
        // directly" intent — derivation happens in ONE place).
        Self::spawn_log_emit(LogEvent {
            kind: ev.kind,
            tenant_id: ev.tenant_id,
            metric_key: ev.metric_key.to_owned(),
            actor_subject: actor,
            attempted_scope: None,
            blocking_scope: None,
            blocking_row_id: None,
            locked_at: ev.locked_at,
            event_at: now,
        });
        Ok(())
    }

    /// Emit a `bypass_attempt` row in its OWN short transaction. The row
    /// MUST commit before the caller returns the 403 (DESIGN §3.6).
    ///
    /// # Errors
    ///
    /// Returns [`AuditPrimarySinkFailure`] when the INSERT or its
    /// surrounding transaction commit fails — the caller MUST translate
    /// this to a 503 `audit_unavailable` envelope instead of the 403,
    /// closing the "audit gap = silent bypass" surface.
    pub async fn emit_bypass_attempt(
        &self,
        db: &DatabaseConnection,
        ctx: &SecurityContext,
        ev: &BypassAttempt<'_>,
    ) -> Result<(), AuditPrimarySinkFailure> {
        let actor = self.tenant_auth.actor_subject(ctx);
        let now = Utc::now();
        // `attempted_values` is always a `json!({...})` value built by
        // `attempted_values_for` — `f64` fields can carry NaN/Inf
        // which serde rejects. Map that failure to a `DbErr::Custom`
        // surfaced through `AuditPrimarySinkFailure` so the caller
        // emits the 503 envelope rather than panicking. In practice
        // the gauntlet's sanity-bound check at `service.rs::sanity_bounds_ok`
        // rejects non-finite values long before we reach here.
        let attempted_values =
            serde_json::to_string(&ev.attempted_values).map_err(|e| AuditPrimarySinkFailure {
                cause: sea_orm::DbErr::Custom(format!(
                    "attempted_values serialization failed: {e}"
                )),
            })?;
        let model = threshold_lock_audit::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7()),
            event_type: ActiveValue::Set(EventKind::BypassAttempt.as_db_str().to_owned()),
            actor_subject: ActiveValue::Set(actor.clone()),
            tenant_id: ActiveValue::Set(ev.tenant_id),
            metric_key: ActiveValue::Set(ev.metric_key.to_owned()),
            attempted_scope: ActiveValue::Set(Some(ev.attempted_scope.as_db_str().to_owned())),
            attempted_values: ActiveValue::Set(Some(attempted_values)),
            blocking_scope: ActiveValue::Set(Some(ev.blocking.scope.as_db_str().to_owned())),
            blocking_row_id: ActiveValue::Set(Some(ev.blocking.id)),
            locked_by: ActiveValue::Set(ev.blocking.locked_by.clone()),
            locked_at: ActiveValue::Set(ev.blocking.locked_at),
            lock_reason: ActiveValue::Set(ev.blocking.lock_reason.clone()),
            event_at: ActiveValue::Set(now),
            created_at: ActiveValue::Set(now),
        };

        // Own short TX so the audit row's persistence is independent of
        // the (rejected) threshold write. The TX boundary makes the
        // failure mode unambiguous: either the row is durable (we return
        // Ok and the caller emits the canonical 403) or it isn't (we
        // return Err and the caller emits the canonical 503).
        let tx = db
            .begin()
            .await
            .map_err(|e| AuditPrimarySinkFailure { cause: e })?;
        if let Err(e) = threshold_lock_audit::Entity::insert(model).exec(&tx).await {
            // Roll back is best-effort; SeaORM's `tx.rollback()` can also
            // fail mid-flight but the connection will reclaim on drop.
            let _ = tx.rollback().await;
            return Err(AuditPrimarySinkFailure { cause: e });
        }
        tx.commit()
            .await
            .map_err(|e| AuditPrimarySinkFailure { cause: e })?;

        // Async log emit — same fire-and-forget contract.
        Self::spawn_log_emit(LogEvent {
            kind: EventKind::BypassAttempt,
            tenant_id: ev.tenant_id,
            metric_key: ev.metric_key.to_owned(),
            actor_subject: actor,
            attempted_scope: Some(ev.attempted_scope),
            blocking_scope: Some(ev.blocking.scope),
            blocking_row_id: Some(ev.blocking.id),
            locked_at: ev.blocking.locked_at,
            event_at: now,
        });
        Ok(())
    }

    /// Spawn a detached task that emits the structured log line. The
    /// task body is trivial (one `tracing::info!`); the spawn dance
    /// exists so a slow log appender (Loki backpressure, ELK pause)
    /// can't block the request thread per DESIGN §3.2 "derived async
    /// stream".
    ///
    /// Associated function (no `&self`) — emitter holds no per-instance
    /// state the spawned task needs; kept on the impl block so callers
    /// route through `Self::spawn_log_emit(...)` and the production /
    /// test seams stay in one place.
    fn spawn_log_emit(ev: LogEvent) {
        tokio::spawn(async move {
            tracing::info!(
                event = "metric_catalog.audit",
                event_type = ev.kind.as_db_str(),
                tenant_id = %ev.tenant_id,
                metric_key = %ev.metric_key,
                actor_subject = %ev.actor_subject,
                attempted_scope = ev.attempted_scope.map_or("", Scope::as_db_str),
                blocking_scope = ev.blocking_scope.map_or("", Scope::as_db_str),
                blocking_row_id = ev.blocking_row_id.map_or_else(String::new, |id| id.to_string()),
                locked_at = ev.locked_at.map_or_else(String::new, |t| t.to_rfc3339()),
                event_at = %ev.event_at.to_rfc3339(),
                "audit-emitter: structured log event"
            );
        });
    }
}

/// Internal helper so the async-spawn closure owns its own copy of every
/// field — no borrowing across the spawn boundary.
struct LogEvent {
    kind: EventKind,
    tenant_id: Uuid,
    metric_key: String,
    actor_subject: String,
    attempted_scope: Option<Scope>,
    blocking_scope: Option<Scope>,
    blocking_row_id: Option<Uuid>,
    locked_at: Option<DateTime<Utc>>,
    event_at: DateTime<Utc>,
}

/// Build the `attempted_values` JSON payload for a `bypass_attempt`
/// audit row. Captures the threshold values the caller was trying to
/// write — pinned as its own helper so the audit row's payload shape
/// can be reviewed in one place.
#[must_use]
pub fn attempted_values_for(
    good: f64,
    warn: f64,
    alert_trigger: Option<f64>,
    alert_bad: Option<f64>,
    is_locked: bool,
    lock_reason: Option<&str>,
) -> serde_json::Value {
    json!({
        "good": good,
        "warn": warn,
        "alert_trigger": alert_trigger,
        "alert_bad": alert_bad,
        "is_locked": is_locked,
        "lock_reason": lock_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_db_strings_match_migration_enum() {
        // Cross-component pin. Migration declares
        // `ENUM('bypass_attempt','lock_set','lock_cleared')` — these
        // strings MUST be byte-identical. A typo here would silently
        // fail every INSERT with a CHECK violation.
        assert_eq!(EventKind::LockSet.as_db_str(), "lock_set");
        assert_eq!(EventKind::LockCleared.as_db_str(), "lock_cleared");
        assert_eq!(EventKind::BypassAttempt.as_db_str(), "bypass_attempt");
    }

    #[test]
    fn attempted_values_shape_round_trips() {
        // Pin the JSON shape that lands in `threshold_lock_audit.attempted_values`.
        // Compliance review queries this column directly (no joins) — a
        // refactor that drops a field would break post-hoc reconstruction
        // of the attempted write.
        let v = attempted_values_for(10.5, 5.25, Some(2.0), None, true, Some("TICKET-1: pin"));
        let good = v["good"]
            .as_f64()
            .unwrap_or_else(|| panic!("good must serialize as f64"));
        assert!((good - 10.5).abs() < f64::EPSILON);
        let warn = v["warn"]
            .as_f64()
            .unwrap_or_else(|| panic!("warn must serialize as f64"));
        assert!((warn - 5.25).abs() < f64::EPSILON);
        let alert_trigger = v["alert_trigger"]
            .as_f64()
            .unwrap_or_else(|| panic!("alert_trigger must serialize as f64"));
        assert!((alert_trigger - 2.0).abs() < f64::EPSILON);
        assert!(v["alert_bad"].is_null());
        assert_eq!(v["is_locked"], json!(true));
        assert_eq!(v["lock_reason"], json!("TICKET-1: pin"));
    }
}
