//! Tenant-override middleware.
//!
//! Under the auth-disabled `api-gateway` host the platform injects a
//! single-tenant `toolkit_security::SecurityContext` (`DEFAULT_TENANT_ID`) into
//! every request's extensions. This thin, stateless middleware reads that
//! context and, when an `X-Insight-Tenant-Id` header is present and parses to
//! a non-nil Uuid, rebuilds the context with that tenant so per-request tenant
//! overrides (used by internal hops / dev tooling) take effect. Handlers and
//! domain services consume `toolkit_security::SecurityContext` exclusively.
//!
//! Mirrors identity's `HeaderTenantContext` so api-gateway / dbt-runner / etc.
//! send the same header to both services.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Header that carries the session-bound tenant on internal hops. Matches
/// `HeaderTenantContext.HeaderName` on the identity service.
pub const TENANT_HEADER: &str = "X-Insight-Tenant-Id";

/// Access scope resolved from the authorization layer.
///
/// Defines which org units and time ranges the user can see.
/// In production, populated by the authz plugin.
/// Currently stubbed to return full access.
#[derive(Debug, Clone)]
pub struct AccessScope {
    /// Org unit IDs the user is allowed to see.
    #[allow(dead_code)] // will be consumed by query engine for row-level filtering
    pub visible_org_unit_ids: Vec<Uuid>,
    // TODO: add effective_from/effective_to per org unit for time-scoped visibility
}

/// Stateless middleware that takes the host-injected `SecurityContext` and,
/// when `X-Insight-Tenant-Id` carries a non-nil Uuid, overrides the tenant by
/// rebuilding the context. Always re-inserts a `SecurityContext` plus the
/// `AccessScope` into the request extensions.
///
/// The host (api-gateway, `auth_disabled = true`) always provides a tenant
/// (`DEFAULT_TENANT_ID`), so there is no unresolved-tenant rejection path here.
pub async fn tenant_middleware(mut req: Request, next: Next) -> Response {
    // Host-injected context (fallback to anonymous if, for some reason, the
    // host didn't inject one — e.g. a direct pod hit on a non-public route).
    let base = req
        .extensions()
        .get::<SecurityContext>()
        .cloned()
        .unwrap_or_else(SecurityContext::anonymous);

    let ctx = match read_session_tenant(&req) {
        Some(tenant_id) => rebuild_with_tenant(&base, tenant_id),
        None => base,
    };

    let scope = resolve_access_scope(&ctx);

    req.extensions_mut().insert(ctx);
    req.extensions_mut().insert(scope);

    next.run(req).await
}

/// Rebuild a `SecurityContext` carrying over subject id/type/scopes but with
/// the overridden `subject_tenant_id`. Falls back to the base context if the
/// builder rejects the inputs (it requires `subject_id` + `subject_tenant_id`,
/// both of which we supply).
fn rebuild_with_tenant(base: &SecurityContext, tenant_id: Uuid) -> SecurityContext {
    let mut builder = SecurityContext::builder()
        .subject_id(base.subject_id())
        .subject_tenant_id(tenant_id)
        .token_scopes(base.token_scopes().to_vec());
    if let Some(subject_type) = base.subject_type() {
        builder = builder.subject_type(subject_type);
    }
    builder.build().unwrap_or_else(|_| base.clone())
}

/// Parses the session-bound tenant from `X-Insight-Tenant-Id`. Rejects:
/// - multi-valued headers (a hostile or misbehaving upstream sending two
///   `X-Insight-Tenant-Id` lines would otherwise silently bind to the first),
/// - `Uuid::nil()` (parseable but non-identity value must not pin tenant context),
/// - any unparseable value.
///
/// Mirrors identity's `HeaderTenantContext.Resolve`.
fn read_session_tenant(req: &Request) -> Option<Uuid> {
    let mut iter = req.headers().get_all(TENANT_HEADER).iter();
    let first = iter.next()?;
    if iter.next().is_some() {
        // More than one value — refuse to pick a winner.
        return None;
    }
    let raw = first.to_str().ok()?;
    Uuid::parse_str(raw.trim()).ok().filter(|id| !id.is_nil())
}

/// Resolve access scope for the given security context.
///
/// # Stub implementation
///
/// Returns unrestricted access. In production, this would:
/// 1. Call authz resolver with `subject_id`
/// 2. Get visible `org_unit_ids` + `effective_from`/`to` per unit
/// 3. Return access scope
fn resolve_access_scope(_ctx: &SecurityContext) -> AccessScope {
    AccessScope {
        visible_org_unit_ids: vec![], // empty = no org filtering (dev mode)
    }
}
