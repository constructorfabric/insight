//! `admin-crud` service — the validation gauntlet for the 5 admin endpoints.
//!
//! Owns the per-write sequence per DESIGN §3.2 admin-crud:
//!
//! 1. **authz** — `is_tenant_admin(ctx.subject_tenant_id(), ctx)`.
//! 2. **referential integrity** — `metric_id` resolves to an enabled
//!    `metric_catalog` row; pull `metric_key` from there (callers never
//!    supply `metric_key` directly — backend-internal name per §3.7).
//! 3. **scope-shape** — `role_slug` / `team_id` sentinels match the
//!    declared `scope`.
//! 4. **sanity bounds** — `warn` direction matches `higher_is_better`.
//! 5. **v1 lock-scope restriction** — `is_locked = true` only on
//!    `product-default` / `tenant`.
//! 6. **`lock_reason` gating** — required on `is_locked = true`; length cap
//!    enforced.
//! 7. **immutable-field check (PUT only)** — `scope` / `role_slug` /
//!    `team_id` rejection when changed.
//! 8. **lock-enforcer** — broader-scope locked-row check.
//! 9. **write** — SeaORM INSERT/UPDATE inside a TX; if it's a lock
//!    transition, the audit row INSERTs in the same TX.
//! 10. **cache-invalidate** — `Standard` for non-lock writes, `Lock` for
//!     `lock_set` / `lock_cleared` (arms the synchronous-bypass window).
//! 11. **schema-validator** — best-effort `validate(metric_key)` after a
//!     successful POST/PUT.
//!
//! Every step that fails returns a fully-formed `axum::response::Response`
//! built through `api::admin::error_map`. The handler layer just passes
//! these through — no envelope construction lives there.

use std::sync::Arc;

use axum::response::Response;
use chrono::Utc;
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::api::admin::error_map::{
    audit_unavailable_response, immutable_field_response, internal_error_response,
    lock_reason_length_response, lock_reason_required_response, lock_scope_invalid_response,
    map_db_err, not_tenant_admin_response, sanity_bound_response, scope_shape_response,
    threshold_locked_response, threshold_not_found_response, unknown_or_disabled_metric_response,
};
use toolkit_security::SecurityContext;
use crate::domain::admin_threshold::audit_emitter::{
    AuditEmitter, BypassAttempt, EventKind, LockTransition, attempted_values_for,
};
use crate::domain::admin_threshold::dto::{
    CreateRequest, ListFilters, ListResponse, Scope, ThresholdView, UpdateRequest,
};
use crate::domain::admin_threshold::lock_enforcer::{CheckTarget, LockCheck, check_broader_locks};
use crate::domain::admin_threshold::repository;
use crate::domain::auth::TenantAuthorization;
use crate::domain::schema_validator::SchemaValidator;
use crate::infra::cache::catalog_cache::{CatalogCache, InvalidateMode};

/// Maximum `lock_reason` length — matches the DB CHECK
/// `chk_metric_threshold_lock_reason_length` and the DESIGN §3.7 line
/// 1020 spec. Enforced at the gauntlet so the user gets a structured
/// rejection without paying a DB round-trip; the DB CHECK is the
/// backstop per `cpt-metric-cat-principle-dual-validate`.
const LOCK_REASON_MAX_LEN: usize = 512;

/// Admin-CRUD service. Cheap to clone — `Arc` over the cache + validator,
/// SeaORM connection is internally `Arc`'d, `AuditEmitter` is `Arc`'d.
#[derive(Clone)]
pub struct AdminThresholdService {
    db: DatabaseConnection,
    tenant_auth: Arc<dyn TenantAuthorization>,
    audit_emitter: AuditEmitter,
    cache: Arc<dyn CatalogCache>,
    validator: SchemaValidator,
}

impl AdminThresholdService {
    #[must_use]
    pub fn new(
        db: DatabaseConnection,
        tenant_auth: Arc<dyn TenantAuthorization>,
        cache: Arc<dyn CatalogCache>,
        validator: SchemaValidator,
    ) -> Self {
        let audit_emitter = AuditEmitter::new(tenant_auth.clone());
        Self {
            db,
            tenant_auth,
            audit_emitter,
            cache,
            validator,
        }
    }

    // ── List / Get ────────────────────────────────────────────────

    /// `GET /v1/admin/metric-thresholds`. `tenant_id` is ALWAYS taken
    /// from `ctx` — never the query string.
    ///
    /// # Errors
    ///
    /// Returns a fully-formed `Response` envelope on rejection.
    pub async fn list(
        &self,
        ctx: &SecurityContext,
        filters: &ListFilters,
    ) -> Result<ListResponse, Response> {
        if !self.tenant_auth.is_tenant_admin(ctx.subject_tenant_id(), ctx) {
            return Err(not_tenant_admin_response());
        }
        let rows = repository::list_thresholds(&self.db, ctx.subject_tenant_id(), filters)
            .await
            .map_err(|e| internal_error_response(&e))?;
        // Resolve `metric_id` + `schema_*` per metric_key. Cache the
        // lookup in a HashMap so a 200-metric tenant pays at most one
        // metric_catalog query per distinct `metric_key`.
        let mut catalog_cache: std::collections::HashMap<String, repository::CatalogJoinRow> =
            std::collections::HashMap::new();
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let cat = if let Some(c) = catalog_cache.get(&row.metric_key) {
                c.clone()
            } else {
                let Some(c) = repository::find_catalog_id_for_metric_key(&self.db, &row.metric_key)
                    .await
                    .map_err(|e| internal_error_response(&e))?
                else {
                    // Orphan threshold row — log + skip rather than emit
                    // a malformed wire entry. The seed migration is the
                    // contract that prevents this in healthy state.
                    tracing::warn!(
                        metric_key = %row.metric_key,
                        "admin-crud list: threshold row's metric_key not found in catalog; skipping"
                    );
                    continue;
                };
                catalog_cache.insert(row.metric_key.clone(), c.clone());
                c
            };
            match repository::view_from_row(&row, &cat) {
                Ok(view) => items.push(view),
                Err(e) => {
                    // Schema drift: skip the row so one bad apple doesn't
                    // 5xx the whole tenant's list. The error path mirrors
                    // the orphan-row branch above — log + count, never
                    // return a partial-row envelope. get_one DOES 5xx
                    // here because the caller addressed THAT row by id.
                    tracing::error!(
                        error = %e,
                        threshold_id = %row.id,
                        "admin-crud list: view_from_row failed (schema drift); skipping row"
                    );
                }
            }
        }
        Ok(ListResponse { items })
    }

    /// `GET /v1/admin/metric-thresholds/{id}`.
    ///
    /// # Errors
    ///
    /// Returns a fully-formed `Response` envelope on rejection.
    pub async fn get_one(
        &self,
        ctx: &SecurityContext,
        id: Uuid,
    ) -> Result<ThresholdView, Response> {
        if !self.tenant_auth.is_tenant_admin(ctx.subject_tenant_id(), ctx) {
            return Err(not_tenant_admin_response());
        }
        let row = repository::find_threshold(&self.db, id)
            .await
            .map_err(|e| internal_error_response(&e))?
            .ok_or_else(|| threshold_not_found_response(id))?;
        if !row_belongs_to_tenant(&row, ctx.subject_tenant_id()) {
            // Cross-tenant read attempt — surface as "not found" rather
            // than "not authorized" so the existence of the id under
            // another tenant doesn't leak (same convention identity
            // services follow). The handler also treats this branch as
            // a 404, not a 403, to preserve that opacity.
            return Err(threshold_not_found_response(id));
        }
        let cat = repository::find_catalog_id_for_metric_key(&self.db, &row.metric_key)
            .await
            .map_err(|e| internal_error_response(&e))?
            .ok_or_else(|| {
                tracing::error!(
                    metric_key = %row.metric_key,
                    threshold_id = %id,
                    "admin-crud get_one: threshold row's metric_key not found in catalog"
                );
                internal_error_response(&sea_orm::DbErr::RecordNotFound(format!(
                    "metric_catalog row for metric_key {} (referenced by threshold {})",
                    row.metric_key, id
                )))
            })?;
        repository::view_from_row(&row, &cat).map_err(|e| internal_error_response(&e))
    }

    // ── Create ────────────────────────────────────────────────────

    /// `POST /v1/admin/metric-thresholds`.
    ///
    /// # Errors
    ///
    /// Returns a fully-formed `Response` envelope on rejection.
    #[allow(clippy::too_many_lines)] // the gauntlet IS the function — splitting hides the order
    pub async fn create(
        &self,
        ctx: &SecurityContext,
        req: &CreateRequest,
    ) -> Result<ThresholdView, Response> {
        // Step 1: authz (pure — trait call, no DB).
        if !self.tenant_auth.is_tenant_admin(ctx.subject_tenant_id(), ctx) {
            return Err(not_tenant_admin_response());
        }

        // Pure validations FIRST — reject malformed requests without
        // spending a DB round-trip. Order matches DESIGN §3.2 admin-crud
        // step list, just with the pure steps lifted ahead of the
        // first DB hit.

        // Step 3: scope-shape + sentinel normalization.
        let role_slug = req.role_slug.clone().unwrap_or_default();
        let team_id = req.team_id.clone().unwrap_or_default();
        if let Some(resp) = validate_scope_shape(req.scope, &role_slug, &team_id) {
            return Err(resp);
        }

        // Step 5: v1 lock-scope restriction.
        if req.is_locked && !matches!(req.scope, Scope::ProductDefault | Scope::Tenant) {
            return Err(lock_scope_invalid_response(None));
        }

        // Step 6: lock_reason gating.
        if let Some(resp) = validate_lock_reason(req.is_locked, req.lock_reason.as_deref(), None) {
            return Err(resp);
        }

        // Step 2: referential integrity. `find_metric_catalog` also
        // returns `higher_is_better`, so step 4 (sanity bounds) is a
        // pure check on the same row — no second DB hit.
        let cat = repository::find_metric_catalog(&self.db, req.metric_id)
            .await
            .map_err(|e| internal_error_response(&e))?;
        let Some(cat) = cat else {
            return Err(unknown_or_disabled_metric_response());
        };
        if !cat.is_enabled {
            return Err(unknown_or_disabled_metric_response());
        }

        // Step 4: sanity bounds (pure check against fetched
        // higher_is_better — same DB round-trip as referential integrity).
        if !sanity_bounds_ok(cat.higher_is_better, req.good, req.warn) {
            return Err(sanity_bound_response());
        }

        // Step 8: lock-enforcer.
        let target = CheckTarget {
            tenant_id: ctx.subject_tenant_id(),
            metric_key: &cat.metric_key,
            scope: req.scope,
        };
        match check_broader_locks(&self.db, target)
            .await
            .map_err(|e| internal_error_response(&e))?
        {
            LockCheck::Clear => {}
            LockCheck::Blocked(blocking) => {
                // DESIGN §3.6 primary-vs-derived sink: audit MUST commit
                // BEFORE the 403. If the audit INSERT fails, the caller
                // gets a 503 instead of the 403 — no silent bypass.
                let attempt = BypassAttempt {
                    tenant_id: ctx.subject_tenant_id(),
                    metric_key: &cat.metric_key,
                    attempted_scope: req.scope,
                    attempted_values: attempted_values_for(
                        req.good,
                        req.warn,
                        req.alert_trigger,
                        req.alert_bad,
                        req.is_locked,
                        req.lock_reason.as_deref(),
                    ),
                    blocking: blocking.clone(),
                };
                if let Err(audit_err) = self
                    .audit_emitter
                    .emit_bypass_attempt(&self.db, ctx, &attempt)
                    .await
                {
                    tracing::error!(
                        cause = %audit_err.cause,
                        tenant_id = %ctx.subject_tenant_id(),
                        metric_key = %cat.metric_key,
                        "audit_emitter.emit_bypass_attempt failed; \
                         surfacing 503 audit_unavailable to avoid silent bypass"
                    );
                    return Err(audit_unavailable_response());
                }
                return Err(threshold_locked_response(&blocking));
            }
        }

        // Step 9: write (inside a TX so the lock-transition audit row
        // commits atomically when `is_locked = true`).
        let tx = repository::begin_tx(&self.db)
            .await
            .map_err(|e| internal_error_response(&e))?;
        let new_id = Uuid::now_v7();
        let actor = self.tenant_auth.actor_subject(ctx);
        let (locked_by, locked_at) = if req.is_locked {
            (Some(actor), Some(Utc::now()))
        } else {
            (None, None)
        };
        let inserted = match repository::insert_threshold(
            &tx,
            new_id,
            ctx.subject_tenant_id(),
            &cat.metric_key,
            req.scope,
            &role_slug,
            &team_id,
            req.good,
            req.warn,
            req.alert_trigger,
            req.alert_bad,
            req.is_locked,
            locked_by.clone(),
            locked_at,
            req.lock_reason.clone(),
        )
        .await
        {
            Ok(row) => row,
            Err(e) => {
                let _ = tx.rollback().await;
                return Err(map_db_err(&e, None));
            }
        };

        if req.is_locked
            && let Err(e) = self
                .audit_emitter
                .emit_lock_transition_in_tx(
                    &tx,
                    ctx,
                    &LockTransition {
                        kind: EventKind::LockSet,
                        tenant_id: ctx.subject_tenant_id(),
                        metric_key: &cat.metric_key,
                        locked_by,
                        locked_at,
                        lock_reason: req.lock_reason.clone(),
                    },
                )
                .await
        {
            // Lock-transition audit row INSERT failed inside the write
            // TX. DESIGN §3.2 audit-emitter primary-vs-derived sink
            // contract is "atomic with the primary write" for lock_set —
            // a row commit without the audit row would silently break
            // the dual-sink invariant. Roll back the threshold write
            // and surface the failure to the caller as a 5xx.
            let _ = tx.rollback().await;
            return Err(internal_error_response(&e));
        }

        if let Err(e) = tx.commit().await {
            return Err(internal_error_response(&e));
        }

        // Step 10: cache invalidate.
        self.invalidate_cache(ctx.subject_tenant_id(), req.is_locked)
            .await;

        // Step 11: schema-validator (best-effort, debounced).
        let _ = self.validator.validate(&cat.metric_key).await;

        // Project the inserted row through the response view. We
        // already know `metric_id` (req-supplied) and the catalog row's
        // schema-status columns from the earlier `find_metric_catalog`
        // — no second lookup.
        let cat_join = repository::CatalogJoinRow {
            id: req.metric_id,
            higher_is_better: cat.higher_is_better,
            schema_status: cat.schema_status,
            schema_error_code: cat.schema_error_code,
        };
        repository::view_from_row(&inserted, &cat_join).map_err(|e| internal_error_response(&e))
    }

    // ── Update ────────────────────────────────────────────────────

    /// `PUT /v1/admin/metric-thresholds/{id}`.
    ///
    /// # Errors
    ///
    /// Returns a fully-formed `Response` envelope on rejection.
    #[allow(clippy::too_many_lines)] // the gauntlet IS the function — splitting hides the order
    pub async fn update(
        &self,
        ctx: &SecurityContext,
        id: Uuid,
        req: &UpdateRequest,
    ) -> Result<ThresholdView, Response> {
        // Step 1: authz (pure).
        if !self.tenant_auth.is_tenant_admin(ctx.subject_tenant_id(), ctx) {
            return Err(not_tenant_admin_response());
        }

        // Pure validation that doesn't depend on the existing row —
        // lock_reason gating (Step 6). Fires before any DB hit so a
        // malformed request (is_locked = true with no reason) doesn't
        // spend a round-trip.
        if let Some(resp) = validate_lock_reason(req.is_locked, req.lock_reason.as_deref(), None) {
            return Err(resp);
        }

        // Load existing row + catalog row. ID-bound writes need the
        // existing row anyway (immutable-field comparison + audit
        // payload); the catalog row carries `higher_is_better` for the
        // sanity-bound check and the `metric_id` / `schema_status` for
        // the response view — folded into one round-trip pair.
        let existing = repository::find_threshold(&self.db, id)
            .await
            .map_err(|e| internal_error_response(&e))?
            .ok_or_else(|| threshold_not_found_response(id))?;

        if !row_belongs_to_tenant(&existing, ctx.subject_tenant_id()) {
            // Cross-tenant write — surface as `not_tenant_admin` per
            // DESIGN §3.3 (both `not_tenant_admin` and the actual
            // tenant-admin-failure converge on the same `reason`).
            return Err(not_tenant_admin_response());
        }

        // Immutable-field check (Step 7) — DESIGN §3.7 line 1034.
        let existing_scope = Scope::from_db_str(&existing.scope).ok_or_else(|| {
            tracing::error!(
                threshold_id = %id,
                scope = %existing.scope,
                "admin-crud update: existing row's scope is not a known enum value (DB drift)"
            );
            internal_error_response(&sea_orm::DbErr::Custom(format!(
                "unknown scope on existing row {id}"
            )))
        })?;
        if let Some(req_scope) = req.scope
            && req_scope != existing_scope
        {
            return Err(immutable_field_response(id, "scope"));
        }
        if let Some(req_role) = req.role_slug.as_deref()
            && req_role != existing.role_slug
        {
            return Err(immutable_field_response(id, "role_slug"));
        }
        if let Some(req_team) = req.team_id.as_deref()
            && req_team != existing.team_id
        {
            return Err(immutable_field_response(id, "team_id"));
        }

        // v1 lock-scope restriction (Step 5) — pure check against the
        // existing row's scope.
        if req.is_locked && !matches!(existing_scope, Scope::ProductDefault | Scope::Tenant) {
            return Err(lock_scope_invalid_response(Some(id)));
        }

        // Catalog row — used for sanity-bound direction now AND for the
        // response-view projection at the end. One query, two consumers.
        let cat = repository::find_catalog_id_for_metric_key(&self.db, &existing.metric_key)
            .await
            .map_err(|e| internal_error_response(&e))?
            .ok_or_else(|| {
                internal_error_response(&sea_orm::DbErr::RecordNotFound(format!(
                    "metric_catalog row for metric_key {} (referenced by threshold {})",
                    existing.metric_key, id
                )))
            })?;

        // Sanity bounds (Step 4) — pure check against the catalog
        // row's `higher_is_better`.
        if !sanity_bounds_ok(cat.higher_is_better, req.good, req.warn) {
            return Err(sanity_bound_response());
        }

        // lock_reason gating (Step 6) with the row id attached for the
        // resource_name on the envelope. The pre-DB pass already
        // rejected malformed requests; this re-runs the gate with
        // `Some(id)` so the wire shape matches the post-DB CHECK arm
        // when the resource is now knowable.
        if let Some(resp) =
            validate_lock_reason(req.is_locked, req.lock_reason.as_deref(), Some(id))
        {
            return Err(resp);
        }

        // Lock-enforcer (Step 8) — runs on EVERY PUT, not just on
        // unlocked→locked transitions. DESIGN §3.2 lock-enforcer
        // "Responsibility scope": "On any pre-write check for a
        // (tenant, metric_key, scope, role_slug, team_id) target, look
        // up locked rows at broader scopes that would shadow the target
        // during resolution." A PUT that changes `good`/`warn` on a
        // role/team row whose broader scope is locked writes values that
        // would be invisible at read time — the resolver halts on the
        // broader lock. That IS a lock-bypass attempt per DESIGN §3.6,
        // and the audit row protects the compliance trail.
        //
        // Self-update on a row whose OWN scope is `product-default` or
        // `tenant` is fine: `broader_scopes()` on those returns either
        // `[]` or `[product-default]`, and the existence of a
        // self-broader lock (the row itself) is filtered out implicitly
        // because the enforcer queries for a row with `is_locked = true`
        // at a *broader* scope — the row's own scope is not broader
        // than itself.
        let target = CheckTarget {
            tenant_id: ctx.subject_tenant_id(),
            metric_key: &existing.metric_key,
            scope: existing_scope,
        };
        if let LockCheck::Blocked(blocking) = check_broader_locks(&self.db, target)
            .await
            .map_err(|e| internal_error_response(&e))?
        {
            let attempt = BypassAttempt {
                tenant_id: ctx.subject_tenant_id(),
                metric_key: &existing.metric_key,
                attempted_scope: existing_scope,
                attempted_values: attempted_values_for(
                    req.good,
                    req.warn,
                    req.alert_trigger,
                    req.alert_bad,
                    req.is_locked,
                    req.lock_reason.as_deref(),
                ),
                blocking: blocking.clone(),
            };
            if let Err(audit_err) = self
                .audit_emitter
                .emit_bypass_attempt(&self.db, ctx, &attempt)
                .await
            {
                tracing::error!(
                    cause = %audit_err.cause,
                    tenant_id = %ctx.subject_tenant_id(),
                    threshold_id = %id,
                    "audit_emitter.emit_bypass_attempt failed on PUT; \
                     surfacing 503 audit_unavailable"
                );
                return Err(audit_unavailable_response());
            }
            return Err(threshold_locked_response(&blocking));
        }

        // Decide audit kind from the lock-state transition.
        let lock_transition_kind = match (existing.is_locked, req.is_locked) {
            (false, true) => Some(EventKind::LockSet),
            (true, false) => Some(EventKind::LockCleared),
            _ => None,
        };

        // Write (Step 9).
        let tx = repository::begin_tx(&self.db)
            .await
            .map_err(|e| internal_error_response(&e))?;

        let actor = self.tenant_auth.actor_subject(ctx);
        let (locked_by, locked_at) = match lock_transition_kind {
            Some(EventKind::LockSet) => (Some(actor.clone()), Some(Utc::now())),
            Some(EventKind::LockCleared) => (None, None),
            // No transition: preserve the existing values verbatim so
            // re-saving an already-locked row doesn't bump `locked_at`.
            _ => (existing.locked_by.clone(), existing.locked_at),
        };
        let lock_reason = if req.is_locked {
            req.lock_reason.clone()
        } else {
            None
        };

        let updated = match repository::update_threshold(
            &tx,
            id,
            req.good,
            req.warn,
            req.alert_trigger,
            req.alert_bad,
            req.is_locked,
            locked_by.clone(),
            locked_at,
            lock_reason.clone(),
        )
        .await
        {
            Ok(row) => row,
            Err(e) => {
                let _ = tx.rollback().await;
                return Err(map_db_err(&e, Some(id)));
            }
        };

        if let Some(kind) = lock_transition_kind
            && let Err(e) = self
                .audit_emitter
                .emit_lock_transition_in_tx(
                    &tx,
                    ctx,
                    &LockTransition {
                        kind,
                        tenant_id: ctx.subject_tenant_id(),
                        metric_key: &existing.metric_key,
                        locked_by: locked_by.clone(),
                        locked_at,
                        lock_reason: lock_reason.clone(),
                    },
                )
                .await
        {
            let _ = tx.rollback().await;
            return Err(internal_error_response(&e));
        }

        if let Err(e) = tx.commit().await {
            return Err(internal_error_response(&e));
        }

        let is_lock_event = lock_transition_kind.is_some();
        self.invalidate_cache(ctx.subject_tenant_id(), is_lock_event)
            .await;

        let _ = self.validator.validate(&existing.metric_key).await;

        // Reuse the catalog row we already loaded before sanity_bounds
        // — schema_status / schema_error_code may have shifted in the
        // meantime if the schema-validator hook above re-checked, but
        // a stale read here is harmless: the next `get_one` will pick
        // up the updated status, and the response just reports what we
        // observed when this write began.
        repository::view_from_row(&updated, &cat).map_err(|e| internal_error_response(&e))
    }

    // ── Delete ────────────────────────────────────────────────────

    /// `DELETE /v1/admin/metric-thresholds/{id}`. Deleting a row whose
    /// `is_locked = true` is treated as a `lock_cleared` transition for
    /// audit purposes — the lock no longer exists.
    ///
    /// # Errors
    ///
    /// Returns a fully-formed `Response` envelope on rejection.
    pub async fn delete(&self, ctx: &SecurityContext, id: Uuid) -> Result<(), Response> {
        if !self.tenant_auth.is_tenant_admin(ctx.subject_tenant_id(), ctx) {
            return Err(not_tenant_admin_response());
        }
        let existing = repository::find_threshold(&self.db, id)
            .await
            .map_err(|e| internal_error_response(&e))?
            .ok_or_else(|| threshold_not_found_response(id))?;
        if !row_belongs_to_tenant(&existing, ctx.subject_tenant_id()) {
            return Err(not_tenant_admin_response());
        }

        let was_locked = existing.is_locked;

        let tx = repository::begin_tx(&self.db)
            .await
            .map_err(|e| internal_error_response(&e))?;
        let deleted = match repository::delete_threshold(&tx, id).await {
            Ok(n) => n,
            Err(e) => {
                let _ = tx.rollback().await;
                return Err(map_db_err(&e, Some(id)));
            }
        };
        if deleted == 0 {
            let _ = tx.rollback().await;
            return Err(threshold_not_found_response(id));
        }
        if was_locked
            && let Err(e) = self
                .audit_emitter
                .emit_lock_transition_in_tx(
                    &tx,
                    ctx,
                    &LockTransition {
                        kind: EventKind::LockCleared,
                        tenant_id: ctx.subject_tenant_id(),
                        metric_key: &existing.metric_key,
                        locked_by: existing.locked_by.clone(),
                        locked_at: existing.locked_at,
                        lock_reason: existing.lock_reason.clone(),
                    },
                )
                .await
        {
            let _ = tx.rollback().await;
            return Err(internal_error_response(&e));
        }

        if let Err(e) = tx.commit().await {
            return Err(internal_error_response(&e));
        }

        self.invalidate_cache(ctx.subject_tenant_id(), was_locked)
            .await;
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────

    async fn invalidate_cache(&self, tenant_id: Uuid, lock_transition: bool) {
        let mode = if lock_transition {
            InvalidateMode::Lock
        } else {
            InvalidateMode::Standard
        };
        if let Err(e) = self.cache.invalidate(tenant_id, mode).await {
            // Cache-invalidate failure is a degradation, not a hard
            // error: the next read on this replica will still serve
            // the updated row (the cache has TTL). Log and continue.
            tracing::warn!(
                error = %e,
                tenant_id = %tenant_id,
                lock_transition,
                "admin-crud: cache invalidate failed; degrading to TTL-based eventual consistency"
            );
        }
    }
}

// ── Free helpers (pure) ──────────────────────────────────────────

/// True iff `row.tenant_id` matches `caller_tenant`. `product-default`
/// rows have `tenant_id = None` and never belong to any tenant for
/// admin-write purposes — admin CRUD on `product-default` is out of
/// scope (DESIGN §3.2 admin-crud: seed-migration owns `product-default`
/// writes).
fn row_belongs_to_tenant(row: &repository::ThresholdRow, caller_tenant: Uuid) -> bool {
    row.tenant_id == Some(caller_tenant)
}

fn validate_scope_shape(scope: Scope, role_slug: &str, team_id: &str) -> Option<Response> {
    // Mirrors the DB CHECK arms `chk_metric_threshold_role_slug_shape`
    // and `chk_metric_threshold_team_id_shape`. The gauntlet returns
    // BOTH violations at once when both are wrong — matches DESIGN §3.3
    // example field_violations shape.
    let role_required = matches!(scope, Scope::Role | Scope::TeamRole);
    let team_required = matches!(scope, Scope::Team | Scope::TeamRole);
    let role_bad = if role_required {
        role_slug.is_empty()
    } else {
        !role_slug.is_empty()
    };
    let team_bad = if team_required {
        team_id.is_empty()
    } else {
        !team_id.is_empty()
    };
    if role_bad || team_bad {
        Some(scope_shape_response(role_bad, team_bad))
    } else {
        None
    }
}

fn sanity_bounds_ok(higher_is_better: bool, good: f64, warn: f64) -> bool {
    // `higher_is_better = true` → larger is better → `good >= warn` is
    // the well-ordered case (good above warn above alert). When
    // `higher_is_better = false` → smaller is better → `good <= warn`.
    if higher_is_better {
        good >= warn
    } else {
        good <= warn
    }
}

fn validate_lock_reason(
    is_locked: bool,
    lock_reason: Option<&str>,
    row_id: Option<Uuid>,
) -> Option<Response> {
    if is_locked {
        let Some(reason) = lock_reason else {
            return Some(lock_reason_required_response(row_id));
        };
        if reason.is_empty() {
            return Some(lock_reason_required_response(row_id));
        }
        if reason.chars().count() > LOCK_REASON_MAX_LEN {
            return Some(lock_reason_length_response(row_id));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    //! Pure-function coverage for the gauntlet helpers. End-to-end
    //! coverage (HTTP → service → DB) lives in `live_tests.rs`.

    use super::*;

    #[test]
    fn scope_shape_valid_for_product_default() {
        // product-default + tenant: role_slug + team_id MUST both be
        // empty-string sentinel.
        assert!(validate_scope_shape(Scope::ProductDefault, "", "").is_none());
        assert!(validate_scope_shape(Scope::Tenant, "", "").is_none());
    }

    #[test]
    fn scope_shape_rejects_role_slug_on_tenant_scope() {
        assert!(validate_scope_shape(Scope::Tenant, "eng", "").is_some());
    }

    #[test]
    fn scope_shape_requires_role_slug_for_role_scope() {
        assert!(validate_scope_shape(Scope::Role, "", "").is_some());
        assert!(validate_scope_shape(Scope::Role, "eng", "").is_none());
        // Role scope MUST NOT carry team_id.
        assert!(validate_scope_shape(Scope::Role, "eng", "alpha").is_some());
    }

    #[test]
    fn scope_shape_requires_both_for_team_role_scope() {
        assert!(validate_scope_shape(Scope::TeamRole, "", "").is_some());
        assert!(validate_scope_shape(Scope::TeamRole, "eng", "").is_some());
        assert!(validate_scope_shape(Scope::TeamRole, "", "alpha").is_some());
        assert!(validate_scope_shape(Scope::TeamRole, "eng", "alpha").is_none());
    }

    #[test]
    fn sanity_bounds_higher_is_better_accepts_good_above_warn() {
        assert!(sanity_bounds_ok(true, 20.0, 10.0));
        assert!(sanity_bounds_ok(true, 10.0, 10.0));
        assert!(!sanity_bounds_ok(true, 10.0, 20.0));
    }

    #[test]
    fn sanity_bounds_lower_is_better_accepts_good_below_warn() {
        // `higher_is_better = false` flips the inequality — common for
        // metrics like "time to resolve" where lower is better.
        assert!(sanity_bounds_ok(false, 10.0, 20.0));
        assert!(sanity_bounds_ok(false, 10.0, 10.0));
        assert!(!sanity_bounds_ok(false, 20.0, 10.0));
    }

    #[test]
    fn lock_reason_required_when_locking() {
        // is_locked = true without a reason → rejected. Empty-string
        // reason gets the same treatment so a UI bug that sends `""`
        // is caught the same way as one that omits the field.
        assert!(validate_lock_reason(true, None, None).is_some());
        assert!(validate_lock_reason(true, Some(""), None).is_some());
        assert!(validate_lock_reason(true, Some("TICKET-1"), None).is_none());
    }

    #[test]
    fn lock_reason_length_cap_enforced() {
        let too_long: String = "a".repeat(LOCK_REASON_MAX_LEN + 1);
        assert!(validate_lock_reason(true, Some(&too_long), None).is_some());
        let at_cap: String = "a".repeat(LOCK_REASON_MAX_LEN);
        assert!(validate_lock_reason(true, Some(&at_cap), None).is_none());
    }

    #[test]
    fn lock_reason_optional_when_unlocked() {
        // is_locked = false → no reason needed (and any provided value
        // is dropped by the service before the write).
        assert!(validate_lock_reason(false, None, None).is_none());
        assert!(validate_lock_reason(false, Some("anything"), None).is_none());
    }

    #[test]
    fn lock_reason_max_len_matches_db_check() {
        // Compile-time-equivalent guard. The DB CHECK
        // `chk_metric_threshold_lock_reason_length` caps `lock_reason`
        // at 512 chars; the app-layer gauntlet (`validate_lock_reason`)
        // uses [`LOCK_REASON_MAX_LEN`] for the same cap. A drift between
        // the two would make the gauntlet accept payloads the DB later
        // rejects — producing a 4xx from the CHECK mapper instead of
        // the gauntlet's structured `OUT_OF_RANGE` envelope. Both fire
        // 400s, but the gauntlet's envelope skips a DB round-trip.
        assert_eq!(
            LOCK_REASON_MAX_LEN, 512,
            "gauntlet LOCK_REASON_MAX_LEN must equal the DB CHECK cap \
             (chk_metric_threshold_lock_reason_length: 512)"
        );
    }

    #[test]
    fn product_default_row_does_not_belong_to_any_tenant() {
        let row = repository::ThresholdRow {
            id: Uuid::nil(),
            tenant_id: None,
            metric_key: "k".to_owned(),
            scope: "product-default".to_owned(),
            role_slug: String::new(),
            team_id: String::new(),
            good: 0.0,
            warn: 0.0,
            alert_trigger: None,
            alert_bad: None,
            is_locked: false,
            locked_by: None,
            locked_at: None,
            lock_reason: None,
        };
        let some_tenant = Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111_u128);
        assert!(!row_belongs_to_tenant(&row, some_tenant));
    }
}
