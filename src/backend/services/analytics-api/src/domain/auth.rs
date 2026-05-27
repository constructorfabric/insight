//! Catalog auth-trait (`cpt-metric-cat-component-auth-trait`).
//!
//! Models the auth dependency as a Rust trait so the catalog's release
//! readiness is not blocked on the Auth service delivery (DESIGN §2.2
//! `cpt-metric-cat-constraint-auth-trait`, §3.2 auth-trait). v1 ships
//! `resolve_tenant` only — `is_tenant_admin` / `actor_subject` arrive in
//! later PRs (#524 / #525) when the read and admin paths consume them.
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
//! ## Security invariant
//!
//! The session-bound tenant ALWAYS wins over the configured default. The
//! default is a fallback, never an override — if a session carries tenant T1
//! and the install is misconfigured with default T2, the resolved tenant is
//! T1, never T2. A bug here is a privilege-escalation bug (cross-tenant
//! disclosure); the unit tests at the bottom of this file exercise that path
//! explicitly.

use uuid::Uuid;

/// Resolves the effective tenant for a request.
///
/// Precedence: `session → configured default → None`. Callers treat `None`
/// as a 400 `invalid_argument` per `cpt-metric-cat-constraint-tenant-default`.
pub trait TenantAuthorization: Send + Sync {
    /// `session_tenant`: the tenant attached to the session by upstream auth
    /// (today: the `X-Insight-Tenant-Id` header stub; eventually the JWT
    /// `insight_tenant_id` claim). `None` when the session carries no tenant.
    fn resolve_tenant(&self, session_tenant: Option<Uuid>) -> Option<Uuid>;
}

/// Configuration-driven implementation: returns the session tenant when
/// present, otherwise falls back to the operator-configured default.
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
}
