//! Integration tests for the tenant-override middleware.
//!
//! Drive a minimal router that mounts `auth::tenant_middleware` (now a
//! stateless `from_fn` layer) in front of a test-only `/_tenant_echo` route and
//! assert on the response. The host (api-gateway, `auth_disabled = true`)
//! injects a `toolkit_security::SecurityContext` into the request extensions;
//! these tests seed that context directly via a preceding layer, then verify:
//!
//! - no `X-Insight-Tenant-Id` header → the injected tenant is preserved,
//! - `X-Insight-Tenant-Id: <uuid>` → the tenant is overridden by the header,
//! - a nil-Uuid header is ignored (does not override).
//!
//! No MariaDB / ClickHouse / Identity client is required — the middleware is
//! pure (header parse + context rebuild).

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::Value;
use toolkit_security::SecurityContext;
use tower::ServiceExt;
use uuid::Uuid;

use crate::auth::{TENANT_HEADER, tenant_middleware};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const INJECTED: Uuid = Uuid::from_u128(0x9999_9999_9999_9999_9999_9999_9999_9999_u128);
const OVERRIDE: Uuid = Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111_u128);

/// Test-only echo handler; reflects the resolved tenant out of the
/// `SecurityContext` so the assertions below can verify it.
async fn tenant_echo(Extension(ctx): Extension<SecurityContext>) -> Json<Value> {
    Json(serde_json::json!({ "tenant_id": ctx.subject_tenant_id() }))
}

/// Build the router with a host-context-injecting layer (simulating the
/// api-gateway host) followed by the tenant-override middleware.
fn router_with_injected(tenant: Uuid) -> Router {
    Router::new()
        .route("/_tenant_echo", get(tenant_echo))
        // `tenant_middleware` runs first (it is the OUTER layer), then the
        // injector. Axum runs layers bottom-to-top on the request path, so the
        // injector (added last) executes first and seeds the host context, then
        // `tenant_middleware` reads/overrides it.
        .layer(from_fn(tenant_middleware))
        .layer(from_fn_with_state(tenant, inject_host_context))
}

/// Stand-in for the host's `SecurityContext` injection: builds a single-tenant
/// context bound to `tenant` and inserts it into the request extensions.
async fn inject_host_context(
    axum::extract::State(tenant): axum::extract::State<Uuid>,
    mut req: Request<Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let Ok(ctx) = SecurityContext::builder()
        .subject_id(Uuid::nil())
        .subject_tenant_id(tenant)
        .build()
    else {
        unreachable!("subject_id + subject_tenant_id are set")
    };
    req.extensions_mut().insert(ctx);
    next.run(req).await
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
async fn no_header_preserves_injected_tenant() -> TestResult {
    // No override header → the host-injected tenant flows through unchanged.
    let app = router_with_injected(INJECTED);

    let resp = app.oneshot(req_get("/_tenant_echo")?).await?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await?;
    assert_eq!(body["tenant_id"], INJECTED.to_string());
    Ok(())
}

#[tokio::test]
async fn header_overrides_injected_tenant() -> TestResult {
    // `X-Insight-Tenant-Id` present → it overrides the host-injected tenant.
    let app = router_with_injected(INJECTED);

    let resp = app
        .oneshot(req_get_with_tenant("/_tenant_echo", OVERRIDE)?)
        .await?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await?;
    assert_eq!(
        body["tenant_id"],
        OVERRIDE.to_string(),
        "header tenant must override the injected tenant"
    );
    Ok(())
}

#[tokio::test]
async fn nil_uuid_header_is_ignored() -> TestResult {
    // Defense in depth: a parseable-but-non-identity tenant value
    // (`Uuid::nil()`) must NOT override the injected tenant.
    let app = router_with_injected(INJECTED);

    let resp = app
        .oneshot(req_get_with_tenant("/_tenant_echo", Uuid::nil())?)
        .await?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await?;
    assert_eq!(body["tenant_id"], INJECTED.to_string());
    Ok(())
}

#[tokio::test]
async fn multi_valued_header_is_ignored() -> TestResult {
    // A hostile/misbehaving upstream sending two `X-Insight-Tenant-Id` values
    // must not silently bind to the first — the override is refused and the
    // injected tenant is preserved.
    let app = router_with_injected(INJECTED);

    let req = Request::builder()
        .uri("/_tenant_echo")
        .method("GET")
        .header(TENANT_HEADER, OVERRIDE.to_string())
        .header(
            TENANT_HEADER,
            Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222_u128).to_string(),
        )
        .body(Body::empty())?;
    let resp = app.oneshot(req).await?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await?;
    assert_eq!(body["tenant_id"], INJECTED.to_string());
    Ok(())
}
