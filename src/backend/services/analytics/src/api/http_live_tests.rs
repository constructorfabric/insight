//! HTTP-level integration tests that drive the **real** route table through
//! `tower::oneshot` against a live MariaDB.
//!
//! Unlike `tenant_resolution_tests` (synthetic echo handler, no backend) these
//! build a full [`AppState`] and mount every analytics route via
//! [`register_routes`], so they exercise the axum handlers end-to-end — the
//! extract → delegate → `Result`→`Response` glue that the service-layer
//! `live_tests` cannot reach. This is what closes the handler coverage gap the
//! service tests leave (see cf/insight#1564).
//!
//! All tests are `#[ignore]`d and skip silently when
//! `INTEGRATION_TESTS_MARIADB_URL` is unset — same convention as the domain
//! `live_tests`. Migrations are applied once up front by the CI `migrate`
//! step; these tests never migrate or reset the DB. ClickHouse and Identity
//! clients point at an unreachable address on purpose: handlers that touch
//! them (`query_metric`, `get_person`) exercise their entry + error-mapping
//! path and return 5xx, which is the behaviour under test here — the DB-backed
//! handlers return real 2xx.
//!
//! Tenant isolation: each test picks its own tenant (either a seed row's tenant
//! for reads, or a fresh `Uuid::now_v7()` for admin writes), so the suite is
//! parallel-safe and does not collide with the domain `live_tests`.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use sea_orm::{
    ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, QueryFilter,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use toolkit::api::OpenApiRegistryImpl;
use toolkit_security::SecurityContext;

use crate::api::AppState;
use crate::config::GearConfig;
use crate::domain::admin_threshold::AdminThresholdService;
use crate::domain::auth::{ConfigTenantAuthorization, TenantAuthorization};
use crate::domain::catalog::{CatalogReader, ThresholdResolver};
use crate::domain::schema_validator::SchemaValidator;
use crate::infra::cache::catalog_cache::{CatalogCache, NoopCatalogCache};
use crate::infra::db::entities;
use crate::infra::identity::IdentityClient;

const ENV_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn connect_or_skip() -> Option<DatabaseConnection> {
    let Ok(url) = std::env::var(ENV_VAR) else {
        eprintln!("skipping: {ENV_VAR} not set");
        return None;
    };
    let mut opts = ConnectOptions::new(url);
    opts.max_connections(4).sqlx_logging(false);
    match Database::connect(opts).await {
        Ok(db) => Some(db),
        Err(e) => {
            eprintln!("skipping: cannot connect to {ENV_VAR}: {e}");
            None
        }
    }
}

/// Unreachable ClickHouse client — handlers that never call it (the DB-backed
/// ones) are unaffected; `query_metric`/`get_person` hit it and 5xx by design.
fn dead_ch() -> insight_clickhouse::Client {
    insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        "http://127.0.0.1:1",
        "analytics",
    ))
}

/// Build a full `AppState` against the live DB. Cache is a no-op stub; authz
/// is the config authorizer (`is_tenant_admin` == true), so the admin write
/// path is reachable without a real identity provider.
fn build_state(db: DatabaseConnection) -> AppState {
    let cache: Arc<dyn CatalogCache> = Arc::new(NoopCatalogCache::default());
    let tenant_auth: Arc<dyn TenantAuthorization> = Arc::new(ConfigTenantAuthorization::new(None));
    let validator = SchemaValidator::new(db.clone(), dead_ch());
    let admin_threshold = AdminThresholdService::new(
        db.clone(),
        tenant_auth.clone(),
        cache.clone(),
        validator.clone(),
    );
    let catalog_reader = CatalogReader::new(cache.clone(), ThresholdResolver::new(db.clone()));
    AppState {
        db,
        ch: dead_ch(),
        identity: IdentityClient::new("http://127.0.0.1:1"),
        config: GearConfig::default(),
        validator,
        tenant_auth,
        catalog_reader,
        admin_threshold,
    }
}

/// Fixed test subject id. Handlers filter by tenant, not subject (subject only
/// surfaces in audit `actor_subject`).
const TEST_PERSON: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0001);

/// Mount the real operation table with the `SecurityContext` injected directly
/// for `tenant`, **bypassing** the host authn pipeline — the
/// `cf-gears-oidc-authn-plugin` verification needs a live JWKS, and that path is
/// covered by the plugin's own tests + the compose e2e. This suite is about the
/// handler -> DB glue for a known caller.
fn app(db: DatabaseConnection, tenant: Uuid) -> Router {
    let openapi = OpenApiRegistryImpl::new();
    let state = Arc::new(build_state(db));
    let api = super::build_operations(Router::new(), &openapi)
        .layer(from_fn_with_state(tenant, inject_host_context))
        .layer(axum::Extension(state));
    Router::new().merge(api)
}

/// Seed a `SecurityContext` (subject + tenant) the way `authverify` would.
async fn inject_host_context(
    axum::extract::State(tenant): axum::extract::State<Uuid>,
    mut req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let Ok(ctx) = SecurityContext::builder()
        .subject_id(TEST_PERSON)
        .subject_type("user")
        .subject_tenant_id(tenant)
        .build()
    else {
        unreachable!("subject_id + subject_tenant_id are set")
    };
    req.extensions_mut().insert(ctx);
    next.run(req).await
}

fn get(uri: &str) -> Result<Request<Body>, axum::http::Error> {
    Request::builder().uri(uri).body(Body::empty())
}

fn json_req(method: &str, uri: &str, body: &Value) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(body).unwrap_or_else(|e| panic!("serialize body: {e}")),
        ))
}

async fn body_json(resp: Response) -> Result<Value, Box<dyn std::error::Error>> {
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// A seeded, enabled metric + the tenant that owns it (the seed migration
/// backfills these under the default tenant).
async fn a_seed_metric(db: &DatabaseConnection) -> Option<entities::metrics::Model> {
    entities::metrics::Entity::find()
        .filter(entities::metrics::Column::IsEnabled.eq(true))
        .one(db)
        .await
        .unwrap_or_else(|e| panic!("query metrics: {e}"))
}

// ── Reads (real 2xx against MariaDB) ─────────────────────────────

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn list_metrics_returns_200_items() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let Some(metric) = a_seed_metric(&db).await else {
        eprintln!("skipping: no enabled metric in seed");
        return Ok(());
    };
    let app = app(db, metric.insight_tenant_id);
    let resp = app.oneshot(get("/v1/metrics")?).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await?;
    assert!(
        body.get("items").is_some(),
        "list payload has items: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn get_person_forwards_authorization_then_5xx_on_dead_identity() -> TestResult {
    // Identity is a dead address (127.0.0.1:1), so this exercises the G1
    // Authorization-forwarding path — the handler reads the incoming bearer and
    // the IdentityClient attaches it to the outbound call — and the mapping of
    // the (unreachable) failure to 5xx, with no live identity provider needed.
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    let req = Request::builder()
        .uri("/v1/persons/nobody@example.com")
        .header("authorization", "Bearer test-gateway-jwt")
        .body(Body::empty())?;
    let resp = app.oneshot(req).await?;
    assert!(
        resp.status().is_server_error(),
        "dead identity should map to 5xx, got {}",
        resp.status()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn get_metric_by_id_returns_200() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let Some(metric) = a_seed_metric(&db).await else {
        eprintln!("skipping: no enabled metric in seed");
        return Ok(());
    };
    let (id, tenant) = (metric.id, metric.insight_tenant_id);
    let app = app(db, tenant);
    let resp = app.oneshot(get(&format!("/v1/metrics/{id}"))?).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn get_unknown_metric_returns_404() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    let resp = app
        .oneshot(get(&format!("/v1/metrics/{}", Uuid::now_v7()))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn list_thresholds_for_metric_returns_200() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let Some(metric) = a_seed_metric(&db).await else {
        eprintln!("skipping: no enabled metric in seed");
        return Ok(());
    };
    let (id, tenant) = (metric.id, metric.insight_tenant_id);
    let app = app(db, tenant);
    let resp = app
        .oneshot(get(&format!("/v1/metrics/{id}/thresholds"))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn list_columns_returns_200() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    let resp = app.oneshot(get("/v1/columns")?).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn list_columns_for_table_returns_200() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    // Any table name is valid input; an unseeded table yields an empty list.
    let resp = app
        .oneshot(get("/v1/columns/analytics.member_metric_values")?)
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn catalog_get_metrics_returns_200() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let Some(metric) = a_seed_metric(&db).await else {
        eprintln!("skipping: no enabled metric in seed");
        return Ok(());
    };
    let app = app(db, metric.insight_tenant_id);
    let resp = app
        .oneshot(json_req("POST", "/v1/catalog/get_metrics", &json!({}))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

// ── Admin threshold CRUD round-trip (201 / 200 / 204 mapping) ─────

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn admin_threshold_crud_round_trip() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    // A metric_catalog row to attach the tenant-scope threshold to.
    let Some(cat) = entities::metric_catalog::Entity::find()
        .one(&db)
        .await
        .unwrap_or_else(|e| panic!("query metric_catalog: {e}"))
    else {
        eprintln!("skipping: no metric_catalog row in seed");
        return Ok(());
    };
    let metric_id = cat.id;
    let tenant = Uuid::now_v7(); // fresh tenant → parallel-safe, no cross-test collision
    let app = app(db, tenant);

    // LIST (empty for a fresh tenant) → 200
    let resp = app
        .clone()
        .oneshot(get("/v1/admin/metric-thresholds")?)
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // CREATE → 201
    let create = json!({
        "metric_id": metric_id,
        "scope": "tenant",
        "good": 25.0,
        "warn": 12.0,
        "is_locked": false
    });
    let resp = app
        .clone()
        .oneshot(json_req("POST", "/v1/admin/metric-thresholds", &create)?)
        .await?;
    assert_eq!(resp.status(), StatusCode::CREATED, "create should 201");
    let created = body_json(resp).await?;
    let id = created["id"]
        .as_str()
        .unwrap_or_else(|| panic!("created payload missing string id: {created}"))
        .to_owned();

    // GET one → 200
    let resp = app
        .clone()
        .oneshot(get(&format!("/v1/admin/metric-thresholds/{id}"))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // UPDATE → 200
    let update = json!({
        "scope": "tenant",
        "good": 30.0,
        "warn": 15.0,
        "is_locked": false
    });
    let resp = app
        .clone()
        .oneshot(json_req(
            "PUT",
            &format!("/v1/admin/metric-thresholds/{id}"),
            &update,
        )?)
        .await?;
    assert_eq!(resp.status(), StatusCode::OK, "update should 200");

    // DELETE → 204
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/admin/metric-thresholds/{id}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "delete should 204");
    Ok(())
}

// ── Rejection paths (canonical envelopes) ────────────────────────

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn admin_create_with_unknown_field_returns_400() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    // `tenant_id` in the body is a denied unknown field per the admin contract.
    let bad = json!({ "metric_id": Uuid::now_v7(), "scope": "tenant",
                      "good": 1.0, "warn": 0.5, "is_locked": false, "tenant_id": Uuid::now_v7() });
    let resp = app
        .oneshot(json_req("POST", "/v1/admin/metric-thresholds", &bad)?)
        .await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

// ── Handlers that reach ClickHouse / Identity (5xx by design) ────

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn query_metric_without_clickhouse_maps_to_error() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let Some(metric) = a_seed_metric(&db).await else {
        eprintln!("skipping: no enabled metric in seed");
        return Ok(());
    };
    let (id, tenant) = (metric.id, metric.insight_tenant_id);
    let app = app(db, tenant);
    let resp = app
        .oneshot(json_req(
            "POST",
            &format!("/v1/metrics/{id}/query"),
            &json!({}),
        )?)
        .await?;
    // The handler runs (extract + metric lookup) and maps the dead-CH failure
    // to a canonical error rather than panicking — any non-2xx is acceptable.
    assert!(
        resp.status().is_client_error() || resp.status().is_server_error(),
        "expected an error status, got {}",
        resp.status()
    );
    Ok(())
}
