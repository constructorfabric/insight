//! Unit tests for the offline OpenAPI generation and the live `/openapi.json`
//! route.
//!
//! [`super::openapi_document`] builds the spec from the exact same route table
//! the live server registers — no database, no HTTP listener — so it exercises
//! `register_operations` + `openapi_info` without a compose stack. Standing up a
//! fake [`super::AppState`] here (unlike the middleware tests in
//! `tenant_resolution_tests`, which need none) is worthwhile: it drives the real
//! [`super::router`] assembly and asserts the served `/openapi.json` route
//! returns the built document. `router` only wires the `axum` graph — it never
//! opens the DB / ClickHouse / Identity connections — so a `Disconnected` DB and
//! non-connecting clients are sufficient.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use super::{AppState, openapi_document, router};
use crate::config::{AppConfig, MetricCatalogConfig};
use crate::domain::admin_threshold::AdminThresholdService;
use crate::domain::auth::{ConfigTenantAuthorization, TenantAuthorization};
use crate::domain::catalog::{CatalogReader, ThresholdResolver};
use crate::domain::schema_validator::SchemaValidator;
use crate::infra::cache::catalog_cache::{CatalogCache, NoopCatalogCache};
use crate::infra::identity::IdentityClient;

#[test]
fn openapi_document_covers_the_route_table() -> anyhow::Result<()> {
    // Build offline (no DB / listener) and inspect the serialized form — the
    // same JSON `print_openapi` emits and the drift gate diffs.
    let doc = openapi_document()?;
    let json = serde_json::to_value(&doc)?;

    // Stable API-contract identity from `openapi_info` (deliberately not the
    // crate version — see the drift-gate rationale).
    assert_eq!(json["info"]["title"], "Analytics API");
    assert_eq!(json["info"]["version"], "1.0.0");

    // Every registered operation shows up as a path.
    let paths = json["paths"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("paths object missing"))?;
    for expected in ["/v1/metrics", "/v1/metrics/queries", "/health"] {
        assert!(paths.contains_key(expected), "missing path {expected}");
    }

    // Typed request/response bodies register real component schemas instead of
    // the pre-migration single generic `object`.
    let schemas = json["components"]["schemas"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("component schemas missing"))?;
    assert!(
        schemas.len() >= 20,
        "expected the typed contract to register many schemas, got {}",
        schemas.len()
    );
    assert!(schemas.contains_key("Metric"), "Metric schema missing");
    Ok(())
}

/// A fully-wired but connection-less [`AppState`] — enough to assemble the
/// router. RULE-DEFAULTS-OK: these are test-fixture constants, not runtime
/// defaults; `router` never dials the DB / ClickHouse / Identity.
fn test_state() -> AppState {
    let db = sea_orm::DatabaseConnection::Disconnected;
    let ch = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        "http://localhost:8123",
        "insight",
    ));
    let cache: Arc<dyn CatalogCache> = Arc::new(NoopCatalogCache::default());
    let validator = SchemaValidator::new(db.clone(), ch.clone());
    let tenant_auth: Arc<dyn TenantAuthorization> = Arc::new(ConfigTenantAuthorization::new(None));
    let catalog_reader = CatalogReader::new(cache.clone(), ThresholdResolver::new(db.clone()));
    let admin_threshold = AdminThresholdService::new(
        db.clone(),
        tenant_auth.clone(),
        cache.clone(),
        validator.clone(),
    );

    let config = AppConfig {
        bind_addr: "127.0.0.1:0".to_owned(),
        database_url: String::new(),
        clickhouse_url: "http://localhost:8123".to_owned(),
        clickhouse_database: "insight".to_owned(),
        clickhouse_user: None,
        clickhouse_password: None,
        identity_url: String::new(),
        redis_url: String::new(),
        metric_catalog: MetricCatalogConfig::default(),
    };

    AppState {
        db,
        ch,
        identity: IdentityClient::new(""),
        config,
        validator,
        tenant_auth,
        catalog_reader,
        admin_threshold,
    }
}

#[tokio::test]
async fn router_serves_the_openapi_document() -> anyhow::Result<()> {
    // `/openapi.json` is public (merged past the tenant layer), so no
    // `X-Insight-Tenant-Id` header is needed.
    let app = router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await?;
    let body: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(body["info"]["title"], "Analytics API");
    assert!(
        body["paths"].get("/v1/metrics").is_some(),
        "served document should include the route table"
    );
    Ok(())
}
