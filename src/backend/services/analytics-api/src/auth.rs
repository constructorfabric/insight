//! Tenant-resolution middleware.
//!
//! Sits in front of every route, extracts the session-bound tenant (today: the
//! `X-Insight-Tenant-Id` header, a stub for the JWT `insight_tenant_id` claim
//! that the gateway-driven flow will eventually inject), and asks the catalog
//! `auth-trait` (`crate::domain::auth::TenantAuthorization`) to resolve it
//! against the operator-configured `metric_catalog.tenant_default_id` fallback
//! per `cpt-metric-cat-constraint-tenant-default` (DESIGN §2.2).
//!
//! When the trait returns `None` — neither session nor configured default —
//! the middleware short-circuits with a canonical `invalid_argument` envelope
//! (`field_violations[{field: "tenant_id", reason: "TENANT_UNRESOLVED"}]`) so
//! every catalog endpoint sees the same rejection shape without re-checking
//! per-handler.
//!
//! Mirrors `src/backend/services/identity/.../HeaderTenantContext.cs` +
//! `ConfigTenantContext.cs` on the identity side so operators get the same
//! single-tenant ergonomic across Insight services.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::TenantError;
use crate::domain::auth::TenantAuthorization;

/// Header that carries the session-bound tenant on internal hops until the
/// JWT-claim path lands. Matches `HeaderTenantContext.HeaderName` on the
/// identity service so api-gateway / dbt-runner / etc. send the same header
/// to both services.
pub const TENANT_HEADER: &str = "X-Insight-Tenant-Id";

/// Security context resolved for the current request.
#[derive(Debug, Clone)]
pub struct SecurityContext {
    /// Authenticated user ID (derived from OIDC `sub` claim).
    #[allow(dead_code)] // will be consumed by authz scope resolution
    pub subject_id: Uuid,
    /// Tenant the request operates on. Populated by `tenant_middleware`
    /// from either the session-bound tenant or the configured single-tenant
    /// fallback; never `Uuid::nil()` on the request path (the middleware
    /// rejects a request with no resolvable tenant).
    pub insight_tenant_id: Uuid,
}

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

/// Middleware that resolves the tenant context and rejects unresolved
/// requests with a canonical `invalid_argument` envelope.
///
/// State is just the `TenantAuthorization` handle — independent from
/// `AppState`, so tests can mount this middleware against the test-only
/// `/_tenant_echo` route without standing up a `DatabaseConnection` or
/// `ClickHouse` client.
pub async fn tenant_middleware(
    State(tenant_auth): State<Arc<dyn TenantAuthorization>>,
    mut req: Request,
    next: Next,
) -> Response {
    let session_tenant = read_session_tenant(&req);

    let Some(tenant_id) = tenant_auth.resolve_tenant(session_tenant) else {
        return tenant_unresolved_response();
    };

    let ctx = SecurityContext {
        // TODO: populate from JWT `sub` claim when JWT validation lands.
        subject_id: Uuid::nil(),
        insight_tenant_id: tenant_id,
    };

    let scope = resolve_access_scope(&ctx);

    req.extensions_mut().insert(ctx);
    req.extensions_mut().insert(scope);

    next.run(req).await
}

/// Parses the session-bound tenant from `X-Insight-Tenant-Id`. Rejects:
/// - multi-valued headers (a hostile or misbehaving upstream sending two
///   `X-Insight-Tenant-Id` lines would otherwise silently bind to the first),
/// - `Uuid::nil()` (parseable but non-identity value must not pin tenant context),
/// - any unparseable value.
///
/// Mirrors identity's `HeaderTenantContext.Resolve` in
/// `src/backend/services/identity/.../HeaderTenantContext.cs`.
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

fn tenant_unresolved_response() -> Response {
    let err: CanonicalError = TenantError::invalid_argument()
        .with_field_violation(
            "tenant_id",
            "Tenant context could not be resolved. Send the \
             X-Insight-Tenant-Id header or configure \
             metric_catalog.tenant_default_id.",
            "TENANT_UNRESOLVED",
        )
        .create();
    err.into_response()
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
