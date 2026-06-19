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
//! ## Admin gate: same-tenant enforced, ROLE still a stub
//!
//! `ConfigTenantAuthorization::is_tenant_admin` enforces a **same-tenant**
//! gate: a session resolved to tenant T is admin for T, and is denied for any
//! other tenant. This closes the cross-tenant privilege-escalation surface at
//! this layer (defense-in-depth alongside the DB-row `tenant_id` check, which
//! already rejects cross-tenant writes with a `not_tenant_admin` envelope).
//!
//! What is STILL a stub is the **role** dimension: every *same-tenant* session
//! is treated as admin (`cpt-metric-cat-constraint-auth-trait` "stub" wording),
//! which unblocks the catalog release and keeps the dev/staging admin surface
//! working. Production MUST swap in the real Auth-service-backed implementor
//! before go-live, otherwise the admin CRUD surface is open to any
//! authenticated *member of the same tenant*. The catalog never relies on the
//! stub for cross-tenant security — that is enforced here and at the row level.
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

use crate::auth::SecurityContext;

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
    /// `session_tenant`: the tenant attached to the session by upstream auth
    /// (today: the `X-Insight-Tenant-Id` header stub; eventually the JWT
    /// `insight_tenant_id` claim). `None` when the session carries no tenant.
    fn resolve_tenant(&self, session_tenant: Option<Uuid>) -> Option<Uuid>;

    /// True iff the caller in `ctx` is authorized as a tenant-admin for
    /// `tenant_id`. The catalog's admin CRUD surface (#525) gates every
    /// write through this. Returning `false` causes the caller to emit a
    /// canonical `permission_denied` envelope with `reason = "not_tenant_admin"`.
    ///
    /// `tenant_id` is the target tenant for the operation — usually
    /// `ctx.insight_tenant_id`, but callers MAY pass the row's
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

    fn is_tenant_admin(&self, tenant_id: Uuid, ctx: &SecurityContext) -> bool {
        // Same-tenant gate. Every caller passes `ctx.insight_tenant_id` (a
        // non-nil, server-resolved tenant), so same-tenant admin operations are
        // unchanged — this stays `true` for them. The CROSS-tenant case (a
        // caller asking to admin a tenant different from the one their session
        // resolved to) is denied here, as defense-in-depth alongside the
        // repository row-tenant check, so no layer certifies cross-tenant admin.
        //
        // The ROLE dimension — is this same-tenant member actually an admin? —
        // is STILL a stub pending the real Auth wiring (every same-tenant
        // session is treated as admin, preserving the dev/staging admin
        // surface). Land that as a separate `TenantAuthorization` implementor.
        !tenant_id.is_nil() && tenant_id == ctx.insight_tenant_id
    }

    fn actor_subject(&self, ctx: &SecurityContext) -> ActorSubject {
        // Stub: surface the placeholder `subject_id` (filled with `Uuid::nil()`
        // by `tenant_middleware` today). When JWT validation lands, the
        // middleware will populate this with the verified `sub` claim and
        // this method passes it through unchanged.
        ctx.subject_id.to_string()
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
        SecurityContext {
            subject_id: subject,
            insight_tenant_id: tenant,
        }
    }

    #[test]
    fn admin_is_same_tenant_only_never_cross_tenant() {
        // SECURITY INVARIANT (red-then-green guard for the cross-tenant
        // privilege-escalation surface): a session resolved to tenant T1 is
        // admin for its OWN tenant but NEVER for a different tenant T2.
        //
        // Previously `is_tenant_admin` returned `true` unconditionally and a
        // unit test pinned that as correct — i.e. CI certified that a T1 caller
        // is admin for T2. This test fails against that stub (red) and passes
        // against the same-tenant gate (green). The role dimension (is this
        // same-tenant member actually an admin?) is still a stub pending real
        // Auth — every same-tenant session is treated as admin, which preserves
        // the dev/staging admin surface; only the cross-tenant escalation is
        // closed here, as defense-in-depth alongside the repository row check.
        let auth = ConfigTenantAuthorization::new(None);
        // same-tenant session is still treated as admin (dev/staging unblock kept)
        assert!(auth.is_tenant_admin(T1, &ctx(T1, Uuid::nil())));
        // cross-tenant MUST be denied
        assert!(!auth.is_tenant_admin(T2, &ctx(T1, Uuid::nil())));
        // a nil target tenant can never be admin
        assert!(!auth.is_tenant_admin(Uuid::nil(), &ctx(T1, Uuid::nil())));
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
