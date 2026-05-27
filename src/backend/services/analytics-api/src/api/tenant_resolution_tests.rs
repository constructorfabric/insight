//! Integration tests for the tenant-resolution middleware (Refs #522).
//!
//! Drive a minimal router that mounts the test-only `/_tenant_echo` route
//! behind `auth::tenant_middleware` and assert on the response — no
//! MariaDB, ClickHouse, or Identity client required. The echo handler just
//! reflects the resolved tenant from `SecurityContext`, which is enough to
//! verify:
//!
//! - the configured single-tenant fallback is used when the request carries
//!   no `X-Insight-Tenant-Id` (single-tenant install),
//! - the canonical `invalid_argument` envelope is returned when neither
//!   session nor configured default resolves (multi-tenant install with a
//!   tenant-less request),
//! - the session-bound tenant ALWAYS wins over the configured default — the
//!   security invariant from `cpt-metric-cat-constraint-tenant-default` and
//!   the DESIGN §3.2 auth-trait boundary. A regression here would be a
//!   cross-tenant disclosure / privilege-escalation bug.
//!
//! Standing up a fake `AppState` would require a real `DatabaseConnection`
//! (sea-orm's workspace features ship only `sqlx-mysql`), which buys nothing
//! at this layer — the middleware does not consult the DB. End-to-end
//! coverage against a live analytics-api with a real DB is tracked
//! separately in #558.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::middleware;
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use crate::auth::{SecurityContext, TENANT_HEADER, tenant_middleware};
use crate::domain::auth::{ConfigTenantAuthorization, TenantAuthorization};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const T1: Uuid = Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111_u128);
const T2: Uuid = Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222_u128);

/// Test-only echo handler; reflects the resolved tenant out of
/// `SecurityContext` so the assertions below can verify it.
async fn tenant_echo(Extension(ctx): Extension<SecurityContext>) -> Json<Value> {
    Json(serde_json::json!({ "tenant_id": ctx.insight_tenant_id }))
}

fn router_with_default(default: Option<Uuid>) -> Router {
    let tenant_auth: Arc<dyn TenantAuthorization> =
        Arc::new(ConfigTenantAuthorization::new(default));

    Router::new()
        .route("/_tenant_echo", get(tenant_echo))
        .layer(middleware::from_fn_with_state(
            tenant_auth,
            tenant_middleware,
        ))
}

fn req_get(uri: &str) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
}

fn req_get_with_tenant(uri: &str, tenant: Uuid) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .uri(uri)
        .method("GET")
        .header(TENANT_HEADER, tenant.to_string())
        .body(Body::empty())
}

async fn body_json(resp: axum::response::Response) -> Result<Value, Box<dyn std::error::Error>> {
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[tokio::test]
async fn single_tenant_fallback_resolves_to_configured_default() -> TestResult {
    // Single-tenant install: operator sets `tenantDefaultId`, request arrives
    // with no `X-Insight-Tenant-Id` header → resolves to the configured T2.
    let app = router_with_default(Some(T2));

    let resp = app.oneshot(req_get("/_tenant_echo")?).await?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await?;
    assert_eq!(body["tenant_id"], T2.to_string());
    Ok(())
}

#[tokio::test]
async fn multi_tenant_no_default_returns_canonical_tenant_unresolved() -> TestResult {
    // Multi-tenant install: no configured default, no header. Middleware MUST
    // short-circuit with the canonical `invalid_argument` envelope carrying
    // `field_violations[{tenant_id, TENANT_UNRESOLVED}]` per §3.3 / RFC 9457.
    let app = router_with_default(None);

    let resp = app.oneshot(req_get("/_tenant_echo")?).await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .ok_or("content-type missing")?;
    assert_eq!(
        ct, "application/problem+json",
        "tenant-unresolved MUST land as application/problem+json (RFC 9457)",
    );

    let body = body_json(resp).await?;
    assert_eq!(
        body["type"],
        "gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~"
    );
    assert_eq!(
        body["context"]["resource_type"],
        "gts.cf.insight.analytics_api.tenant.v1~"
    );

    let violations = body["context"]["field_violations"]
        .as_array()
        .ok_or("field_violations must be an array")?;
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0]["field"], "tenant_id");
    assert_eq!(violations[0]["reason"], "TENANT_UNRESOLVED");
    Ok(())
}

#[tokio::test]
async fn session_tenant_wins_over_configured_default() -> TestResult {
    // SECURITY INVARIANT (cpt-metric-cat-constraint-tenant-default,
    // DESIGN §3.2 auth-trait boundary): when a session is bound to T1 and the
    // install is configured with a *different* default T2, the resolved
    // tenant MUST be T1. The default is a fallback, never an override. A
    // regression here is a cross-tenant disclosure / privilege-escalation
    // bug.
    let app = router_with_default(Some(T2));

    let resp = app
        .oneshot(req_get_with_tenant("/_tenant_echo", T1)?)
        .await?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await?;
    assert_eq!(
        body["tenant_id"],
        T1.to_string(),
        "session tenant T1 must win over configured default T2"
    );
    Ok(())
}

#[tokio::test]
async fn nil_uuid_header_is_treated_as_unset() -> TestResult {
    // Defense in depth: a parseable-but-non-identity tenant value
    // (`Uuid::nil()`) must not pin tenant context. Mirrors identity's
    // `HeaderTenantContext.Resolve` check. Without a configured default this
    // means the request still fails with `TENANT_UNRESOLVED` — the nil
    // header does NOT count as "session has a tenant".
    let app = router_with_default(None);

    let resp = app
        .oneshot(req_get_with_tenant("/_tenant_echo", Uuid::nil())?)
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await?;
    assert_eq!(
        body["context"]["field_violations"][0]["reason"],
        "TENANT_UNRESOLVED"
    );
    Ok(())
}

#[tokio::test]
async fn multi_valued_tenant_header_is_refused() -> TestResult {
    // A hostile or misbehaving upstream sending two `X-Insight-Tenant-Id`
    // values must not silently bind to the first. Without a configured
    // default the middleware MUST reject — picking either value would be a
    // smuggling vector.
    let app = router_with_default(None);

    let req = Request::builder()
        .uri("/_tenant_echo")
        .method("GET")
        .header(TENANT_HEADER, T1.to_string())
        .header(TENANT_HEADER, T2.to_string())
        .body(Body::empty())?;
    let resp = app.oneshot(req).await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await?;
    assert_eq!(
        body["context"]["field_violations"][0]["reason"],
        "TENANT_UNRESOLVED"
    );
    Ok(())
}
