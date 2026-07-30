//! Catalog auth-trait (`cpt-metric-cat-component-auth-trait`).
//!
//! Models the auth dependency as a Rust trait so the catalog's release
//! readiness is not blocked on the Auth service delivery (DESIGN §2.2
//! `cpt-metric-cat-constraint-auth-trait`, §3.2 auth-trait). The trait
//! surface mirrors what catalog components need:
//!
//! - `resolve_tenant` (Refs #522) — resolves the request's effective tenant.
//! - `is_tenant_admin` (Refs #525) — gates the admin write path.
//! - `actor_subject` (Refs #525) — populates `threshold_lock_audit.actor_subject`.
//!
//! ## Single-tenant fallback (`cpt-metric-cat-constraint-tenant-default`)
//!
//! Mirrors the identity service's `ConfigTenantContext`
//! (`src/backend/services/identity/src/Insight.Identity.Api/Auth/ConfigTenantContext.cs`):
//! when the request arrives without a session-bound tenant, the configured
//! `metric_catalog.tenant_default_id` (env
//! `ANALYTICS__metric_catalog__tenant_default_id`) is used; multi-tenant
//! installs leave it unset and tenant-less requests fail with a canonical
//! `invalid_argument` envelope carried by `TENANT_UNRESOLVED`.
//!
//! ## Admin gate is a STUB until real Auth wires in
//!
//! `ConfigTenantAuthorization::is_tenant_admin` returns `true` for every
//! resolved session. This matches the DESIGN's literal "stub" wording
//! (`cpt-metric-cat-constraint-auth-trait`) and unblocks the catalog
//! release; production deployment MUST swap this implementation for the
//! real Auth-service-backed one before going live, otherwise the admin
//! CRUD surface is open to any authenticated tenant member. The catalog
//! never relies on the stub being correct for security; the admin path is
//! also defended at the DB-row level (cross-tenant writes are rejected
//! because the row's `tenant_id` mismatch surfaces a `not_tenant_admin`
//! envelope regardless of what `is_tenant_admin` returns).
//!
//! ## Security invariant
//!
//! The session-bound tenant ALWAYS wins over the configured default. The
//! default is a fallback, never an override — if a session carries tenant T1
//! and the install is misconfigured with default T2, the resolved tenant is
//! T1, never T2. A bug here is a privilege-escalation bug (cross-tenant
//! disclosure); the unit tests at the bottom of this file exercise that path
//! explicitly.

use uuid::Uuid;

use toolkit_security::SecurityContext;

/// Stable principal identifier used in `threshold_lock_audit.actor_subject`.
/// Distinct type from `Uuid::sub` / arbitrary header value so a future swap
/// to the real Auth wiring (which surfaces an opaque `sub` claim) is a
/// trait-level change instead of a string-typed signature drift.
pub type ActorSubject = String;

/// Resolves the effective tenant for a request and adjudicates admin authz
/// + audit-actor identity for catalog components.
///
/// Tenant precedence: `session → configured default → None`. Callers treat
/// `None` as a 400 `invalid_argument` per
/// `cpt-metric-cat-constraint-tenant-default`.
pub trait TenantAuthorization: Send + Sync {
    /// `session_tenant`: the request's resolved tenant. It comes from the signed
    /// gateway JWT — the `authverify` layer selects it from the JWT's `tenants[]`
    /// via the `X-Tenant-ID` selector (G2) and sets `SecurityContext`'s
    /// `subject_tenant_id`. `None` when the caller carries no tenant.
    ///
    /// Not called from the request path today (`authverify` sets the context's
    /// tenant directly), but retained on the trait + covered by unit tests for
    /// the single-tenant-fallback security invariant.
    #[allow(dead_code)]
    fn resolve_tenant(&self, session_tenant: Option<Uuid>) -> Option<Uuid>;

    /// True iff the caller in `ctx` is authorized as a tenant-admin for
    /// `tenant_id`. The catalog's admin CRUD surface (#525) gates every
    /// write through this. Returning `false` causes the caller to emit a
    /// canonical `permission_denied` envelope with `reason = "not_tenant_admin"`.
    ///
    /// `tenant_id` is the target tenant for the operation — usually
    /// `ctx.subject_tenant_id()`, but callers MAY pass the row's
    /// `tenant_id` to catch cross-tenant writes here too. v1 stub does not
    /// distinguish the two; both routes converge on the same DB-row
    /// tenant check at the repository layer.
    fn is_tenant_admin(&self, tenant_id: Uuid, ctx: &SecurityContext) -> bool;

    /// Stable principal identifier for `ctx`. Surfaced in
    /// `threshold_lock_audit.actor_subject` and the structured-log stream
    /// (DESIGN §3.7 — explicitly NOT a session token; sessions rotate but
    /// audit retention is ≥ 1 year).
    fn actor_subject(&self, ctx: &SecurityContext) -> ActorSubject;
}

/// Configuration-driven implementation: returns the session tenant when
/// present, otherwise falls back to the operator-configured default. The
/// admin gate is a stub (see module doc-comment).
pub struct ConfigTenantAuthorization {
    // Read only by `resolve_tenant`, which is no longer on the request path
    // under the auth-disabled host (see the trait method's note). Retained for
    // the single-tenant fallback + its unit tests.
    #[allow(dead_code)]
    default: Option<Uuid>,
}

impl ConfigTenantAuthorization {
    /// Filters `Some(Uuid::nil())` out of the configured default — same
    /// reasoning as the header path in `auth::tenant_middleware`: a
    /// parseable-but-non-identity value MUST NOT pin tenant context. Lets
    /// the middleware's `SecurityContext.insight_tenant_id != nil`
    /// invariant hold even if an operator misconfigures the Helm value to
    /// the zero UUID. Mirrors identity's `HeaderTenantContext.Resolve`
    /// nil-rejection.
    #[must_use]
    pub fn new(default: Option<Uuid>) -> Self {
        Self {
            default: default.filter(|id| !id.is_nil()),
        }
    }
}

impl TenantAuthorization for ConfigTenantAuthorization {
    fn resolve_tenant(&self, session_tenant: Option<Uuid>) -> Option<Uuid> {
        // `or` short-circuits: when the session carries `Some(_)`, the
        // configured default is never consulted. This is the security
        // invariant from `cpt-metric-cat-constraint-tenant-default` — see
        // the module doc-comment and the `session_wins_over_configured_default`
        // unit test below.
        session_tenant.or(self.default)
    }

    fn is_tenant_admin(&self, _tenant_id: Uuid, _ctx: &SecurityContext) -> bool {
        // Stub: every resolved session is treated as tenant-admin until the
        // real Auth wiring lands. See module doc-comment.
        true
    }

    fn actor_subject(&self, ctx: &SecurityContext) -> ActorSubject {
        // Stub: surface the placeholder `subject_id` (filled with `Uuid::nil()`
        // by `tenant_middleware` today). When JWT validation lands, the
        // middleware will populate this with the verified `sub` claim and
        // this method passes it through unchanged.
        ctx.subject_id().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T1: Uuid = Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111_u128);
    const T2: Uuid = Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222_u128);

    #[test]
    fn session_tenant_resolves_when_present() {
        let auth = ConfigTenantAuthorization::new(None);
        assert_eq!(auth.resolve_tenant(Some(T1)), Some(T1));
    }

    #[test]
    fn falls_back_to_configured_default() {
        let auth = ConfigTenantAuthorization::new(Some(T2));
        assert_eq!(auth.resolve_tenant(None), Some(T2));
    }

    #[test]
    fn session_wins_over_configured_default() {
        // Security invariant: a misconfigured install with default=T2 must
        // NEVER override a request whose session is bound to T1. This is the
        // privilege-escalation surface that the single-tenant fallback opens
        // up — every change to this resolver MUST keep this test green.
        let auth = ConfigTenantAuthorization::new(Some(T2));
        assert_eq!(auth.resolve_tenant(Some(T1)), Some(T1));
    }

    #[test]
    fn unresolved_when_neither() {
        let auth = ConfigTenantAuthorization::new(None);
        assert_eq!(auth.resolve_tenant(None), None);
    }

    #[test]
    fn nil_configured_default_is_treated_as_unset() {
        // Defense in depth: the header path filters `Uuid::nil()` (see
        // `auth::read_session_tenant`). A misconfigured Helm value with
        // `tenant_default_id: 00000000-0000-0000-0000-000000000000` must
        // get the same treatment, so the `SecurityContext.insight_tenant_id
        // != nil` post-middleware invariant holds against both inputs.
        let auth = ConfigTenantAuthorization::new(Some(Uuid::nil()));
        assert_eq!(auth.resolve_tenant(None), None);
        assert_eq!(auth.resolve_tenant(Some(T1)), Some(T1));
    }

    fn ctx(tenant: Uuid, subject: Uuid) -> SecurityContext {
        // Builder requires subject_id + subject_tenant_id; both are set, so
        // `build()` cannot fail. `unreachable!` on the Err arm keeps this
        // helper's return type `SecurityContext` without tripping the
        // workspace `expect_used` / `unwrap_used` deny lints.
        let Ok(ctx) = SecurityContext::builder()
            .subject_id(subject)
            .subject_tenant_id(tenant)
            .build()
        else {
            unreachable!("subject_id + subject_tenant_id are set")
        };
        ctx
    }

    #[test]
    fn stub_grants_admin_for_every_resolved_session() {
        // The v1 stub returns `true` unconditionally. This pins that
        // behaviour so a refactor that adds gating logic without wiring
        // the real Auth backend trips the test — silently flipping to
        // "deny by default" would brick the admin surface in dev/staging
        // and silently change production behaviour the moment real Auth
        // lands. The right path is to land the real impl as a separate
        // `TenantAuthorization` implementor, not to reshape the stub.
        let auth = ConfigTenantAuthorization::new(None);
        assert!(auth.is_tenant_admin(T1, &ctx(T1, Uuid::nil())));
        // Even when target tenant ≠ session tenant the stub returns true;
        // cross-tenant rejection lives at the row-tenant check in the
        // admin repository, not in the stub.
        assert!(auth.is_tenant_admin(T2, &ctx(T1, Uuid::nil())));
    }

    #[test]
    fn actor_subject_passes_through_security_context_subject_id() {
        // Today `subject_id` is `Uuid::nil()` (filled by `tenant_middleware`
        // until JWT validation lands). When real JWT lands the middleware
        // populates `subject_id` with the verified `sub` claim and this
        // method passes it through unchanged — audit rows / log lines pick
        // up the real principal automatically.
        let auth = ConfigTenantAuthorization::new(None);
        let subject = Uuid::from_u128(0x9999_9999_9999_9999_9999_9999_9999_9999_u128);
        assert_eq!(
            auth.actor_subject(&ctx(T1, subject)),
            subject.to_string(),
            "actor_subject MUST reflect ctx.subject_id verbatim"
        );
    }
}
