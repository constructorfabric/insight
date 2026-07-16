//! Tenant-override middleware.
//!
//! The auth-disabled `api-gateway` host injects a single-tenant
//! `toolkit_security::SecurityContext` (`DEFAULT_TENANT_ID`) into every
//! request. This thin, stateless middleware reads it and, when an
//! `X-Insight-Tenant-Id` header carries a non-nil Uuid, rebuilds the context
//! with that tenant so handlers resolve against the right tenant. Mirrors the
//! analytics gear + identity's `HeaderTenantContext`.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Header carrying the session-bound tenant on internal hops.
pub const TENANT_HEADER: &str = "X-Insight-Tenant-Id";

/// Reads the host-injected `SecurityContext` and overrides its tenant from
/// `X-Insight-Tenant-Id` when present, then re-inserts it for handlers.
pub async fn tenant_middleware(mut req: Request, next: Next) -> Response {
    let base = req
        .extensions()
        .get::<SecurityContext>()
        .cloned()
        .unwrap_or_else(SecurityContext::anonymous);

    let ctx = match read_session_tenant(&req) {
        Some(tenant_id) => rebuild_with_tenant(&base, tenant_id),
        None => base,
    };

    req.extensions_mut().insert(ctx);
    next.run(req).await
}

/// Rebuild a `SecurityContext` carrying over subject id/type/scopes but with the
/// overridden `subject_tenant_id`. Falls back to the base context on failure.
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

/// Parse the tenant from `X-Insight-Tenant-Id`. Rejects multi-valued headers,
/// `Uuid::nil()`, and unparseable values.
fn read_session_tenant(req: &Request) -> Option<Uuid> {
    let mut iter = req.headers().get_all(TENANT_HEADER).iter();
    let first = iter.next()?;
    if iter.next().is_some() {
        return None;
    }
    let raw = first.to_str().ok()?;
    Uuid::parse_str(raw.trim()).ok().filter(|id| !id.is_nil())
}
