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
//! them exercise their entry + error-mapping path and return 5xx, which is the
//! behaviour under test here — the DB-backed handlers return real 2xx.
//!
//! Tenant isolation: each test picks its own tenant (either a seed row's tenant
//! for reads, or a fresh `Uuid::now_v7()` for writes), so the suite is
//! parallel-safe and does not collide with the domain `live_tests`.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use toolkit::api::OpenApiRegistryImpl;
use toolkit_security::SecurityContext;

use crate::api::AppState;
use crate::config::GearConfig;
use crate::domain::metric_definitions::test_fixture::DrilldownFixture;
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
/// ones) are unaffected; the ones that do hit it 5xx by design.
fn dead_ch() -> insight_clickhouse::Client {
    insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        "http://127.0.0.1:1",
        "analytics",
    ))
}

/// An Anthropic client no test reaches: the AI routes under test refuse before
/// any upstream call.
fn dead_anthropic() -> crate::infra::anthropic::AnthropicClient {
    #[expect(
        clippy::expect_used,
        reason = "a client with no proxy config cannot fail to build"
    )]
    crate::infra::anthropic::AnthropicClient::new(
        "http://127.0.0.1:1",
        std::time::Duration::from_secs(1),
    )
    .expect("client builds")
}

/// Build a full `AppState` against the live DB.
fn build_state_with_ch(
    db: DatabaseConnection,
    identity: IdentityClient,
    ch: insight_clickhouse::Client,
) -> AppState {
    AppState {
        db,
        ch,
        identity,
        anthropic: dead_anthropic(),
        ai_calls: Arc::new(tokio::sync::Semaphore::new(1)),
        config: GearConfig::default(),
        external_links: crate::domain::external_links::ExternalSourceRegistry::default(),
    }
}

/// A live ClickHouse client from the same env the domain `live_tests` use;
/// `None` (skip) when unset or empty.
fn live_ch_or_skip() -> Option<insight_clickhouse::Client> {
    let url = std::env::var("INTEGRATION_TESTS_CLICKHOUSE_URL").unwrap_or_default();
    if url.is_empty() {
        eprintln!("skipping: INTEGRATION_TESTS_CLICKHOUSE_URL not set");
        return None;
    }
    let mut config = insight_clickhouse::Config::new(url, "default");
    if let (Ok(user), Ok(password)) = (
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER"),
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD"),
    ) && !user.is_empty()
    {
        config = config.with_auth(user, password);
    }
    Some(insight_clickhouse::Client::new(config))
}

/// Fixed test subject id. Handlers filter by tenant, not subject (subject only
/// surfaces in audit `actor_subject`).
const TEST_PERSON: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0001);
/// A person the loopback identity reports as visible to the caller.
const VISIBLE_PERSON: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0002);
/// A person it does not — the gate's 403 case.
const HIDDEN_PERSON: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0003);

/// Mount the real operation table with the `SecurityContext` injected directly
/// for `tenant`, **bypassing** the host authn pipeline — the
/// `cf-gears-oidc-authn-plugin` verification needs a live JWKS, and that path is
/// covered by the plugin's own tests + the compose e2e. This suite is about the
/// handler -> DB glue for a known caller.
fn app(db: DatabaseConnection, tenant: Uuid) -> Router {
    let Ok(identity) = IdentityClient::new("http://127.0.0.1:1") else {
        unreachable!("the static identity url builds a client")
    };
    app_with_identity(db, tenant, identity)
}

fn app_with_identity(db: DatabaseConnection, tenant: Uuid, identity: IdentityClient) -> Router {
    app_with_identity_and_ch(db, tenant, identity, dead_ch())
}

fn app_with_identity_and_ch(
    db: DatabaseConnection,
    tenant: Uuid,
    identity: IdentityClient,
    ch: insight_clickhouse::Client,
) -> Router {
    let openapi = OpenApiRegistryImpl::new();
    let state = Arc::new(build_state_with_ch(db, identity, ch));
    let api = super::build_operations(Router::new(), &openapi)
        .layer(from_fn_with_state(tenant, inject_host_context))
        .layer(axum::Extension(state));
    Router::new().merge(api)
}

/// Loopback identity serving `POST /v1/visible-persons` — answers with the
/// intersection of the requested person ids and `visible` — and `GET /v1/me`
/// reporting no roles (a non-admin caller).
async fn spawn_identity(visible: &[Uuid]) -> Result<IdentityClient, Box<dyn std::error::Error>> {
    spawn_identity_with_roles(visible, false).await
}

/// The seeded identity `admin` role id mirrored from `infra::identity`.
const ADMIN_ROLE_ID: &str = "a4d11000-0000-4000-8000-000000000001";

async fn spawn_identity_with_roles(
    visible: &[Uuid],
    admin: bool,
) -> Result<IdentityClient, Box<dyn std::error::Error>> {
    let visible = Arc::new(
        visible
            .iter()
            .copied()
            .collect::<std::collections::HashSet<Uuid>>(),
    );

    let roles = if admin {
        json!([{"role_id": ADMIN_ROLE_ID}])
    } else {
        json!([])
    };
    let app = Router::new()
        .route(
            "/v1/visible-persons",
            axum::routing::post(move |axum::Json(req): axum::Json<Value>| {
                let visible = Arc::clone(&visible);
                async move {
                    let granted = req["person_ids"]
                        .as_array()
                        .map(|ids| {
                            ids.iter()
                                .filter_map(|v| v.as_str())
                                .filter_map(|v| Uuid::parse_str(v).ok())
                                .filter(|person_id| visible.contains(person_id))
                                .map(|person_id| person_id.to_string())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    axum::Json(json!({"visible": granted}))
                }
            }),
        )
        .route(
            "/v1/me",
            axum::routing::get(move || {
                let roles = roles.clone();
                async move { axum::Json(json!({"roles": roles})) }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(IdentityClient::new(&format!("http://{addr}"))?)
}

fn metric_results_body(person_ids: &[Uuid]) -> Value {
    let ids = person_ids.iter().map(Uuid::to_string).collect::<Vec<_>>();
    json!({
        "entity": {"type": "person", "ids": ids},
        "period": {"from": "2026-01-01", "to": "2026-01-31"},
        "metrics": [{"metric_key": "ai.accepted_lines", "views": [{"view": "period"}]}],
    })
}

fn drilldown_body(person_id: Uuid) -> Value {
    json!({
        "metric_key": "git.commits",
        "entity": {"type": "person", "id": person_id.to_string()},
        "period": {"from": "2026-07-01", "to": "2026-07-28"},
        "limit": 100
    })
}

fn drilldown_export_body(person_id: Uuid) -> Value {
    json!({
        "metric_key": "git.commits",
        "entity": {"type": "person", "id": person_id.to_string()},
        "period": {"from": "2026-07-01", "to": "2026-07-28"},
        "filters": [],
        "display_dimensions": [],
        "format": "csv"
    })
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
        // The gateway always forwards a bearer on authenticated routes; handlers
        // that fan out to identity read it off the request.
        .header("authorization", "Bearer test-gateway-jwt")
        .body(Body::from(
            serde_json::to_vec(body).unwrap_or_else(|e| panic!("serialize body: {e}")),
        ))
}

async fn body_json(resp: Response) -> Result<Value, Box<dyn std::error::Error>> {
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

// ── Reads (real 2xx against MariaDB) ─────────────────────────────

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn metric_results_forbids_a_person_outside_the_callers_visible_set() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let identity = spawn_identity(&[VISIBLE_PERSON]).await?;
    let app = app_with_identity(db, Uuid::now_v7(), identity);

    let req = json_req(
        "POST",
        "/v1/metric-results",
        &metric_results_body(&[HIDDEN_PERSON]),
    )?;
    let resp = app.oneshot(req).await?;

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a person the caller cannot see must not resolve to data"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn metric_results_does_not_deny_a_person_inside_the_callers_visible_set() -> TestResult {
    // ClickHouse is unreachable here, so an admitted request answers 200 with
    // an error view in the metric's slot; what this pins is that the gate did
    // not reject it. Asserting the error view (not merely `!= 403`) is
    // deliberate: a 400 from validation would satisfy a not-forbidden
    // assertion and hide a request that never reached the gate at all.
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let identity = spawn_identity(&[VISIBLE_PERSON]).await?;
    let app = app_with_identity(db, Uuid::now_v7(), identity);

    let req = json_req(
        "POST",
        "/v1/metric-results",
        &metric_results_body(&[VISIBLE_PERSON]),
    )?;
    let resp = app.oneshot(req).await?;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a visible person must pass the gate; the unreachable ClickHouse \
         answers inside the view, not as a request error"
    );
    let body = body_json(resp).await?;
    assert_eq!(
        body["metrics"][0]["views"][0]["view"], "error",
        "the admitted request must have reached ClickHouse and failed there"
    );
    Ok(())
}

// Both drilldown routes gate before validation, so neither test needs a
// `DrilldownFixture`: the metric definition is never loaded. A 404 here would
// mean the gate had moved back behind validation.

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn metric_drilldown_forbids_a_person_outside_the_callers_visible_set() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let identity = spawn_identity(&[VISIBLE_PERSON]).await?;
    let app = app_with_identity(db, Uuid::now_v7(), identity);

    let req = json_req(
        "POST",
        "/v1/metric-drilldown",
        &drilldown_body(HIDDEN_PERSON),
    )?;
    let resp = app.oneshot(req).await?;

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a person the caller cannot see must not resolve to evidence rows"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn metric_drilldown_export_forbids_a_person_outside_the_callers_visible_set() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let identity = spawn_identity(&[VISIBLE_PERSON]).await?;
    let app = app_with_identity(db, Uuid::now_v7(), identity);

    let req = json_req(
        "POST",
        "/v1/metric-drilldown/export",
        &drilldown_export_body(HIDDEN_PERSON),
    )?;
    let resp = app.oneshot(req).await?;

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "the export route serves the same per-person evidence as the page route \
         and must deny the same callers"
    );
    Ok(())
}

// ── Saved-query CRUD + run (#1965) ───────────────────────────────
//
// Covers every non-5xx response of `/v1/queries*`. CRUD is DB-only, so the
// happy paths return real 2xx against MariaDB. `/run` reaches ClickHouse; with
// `dead_ch` its 200 collapses to 5xx (out of scope here), so only its 404
// (unknown id, resolved before any CH call) is asserted. Each test uses a fresh
// `Uuid::now_v7()` tenant, so the empty-to-populated `saved_queries` table is
// parallel-safe and needs no seed row.

fn create_body() -> Value {
    json!({ "name": "coverage query", "description": "d", "sql": "SELECT 1" })
}

fn delete_req(uri: &str) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .body(Body::empty())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn saved_query_crud_round_trip() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();
    let app = app(db, tenant);

    // CREATE → 201
    let resp = app
        .clone()
        .oneshot(json_req("POST", "/v1/queries", &create_body())?)
        .await?;
    assert_eq!(resp.status(), StatusCode::CREATED, "create should 201");
    let created = body_json(resp).await?;
    let id = created["id"]
        .as_str()
        .unwrap_or_else(|| panic!("created payload missing string id: {created}"))
        .to_owned();

    // LIST → 200, contains the new row
    let resp = app.clone().oneshot(get("/v1/queries")?).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let list = body_json(resp).await?;
    let items = list["items"]
        .as_array()
        .unwrap_or_else(|| panic!("list payload has no items array: {list}"));
    assert!(
        items.iter().any(|i| i["id"].as_str() == Some(id.as_str())),
        "list should contain the created query: {list}"
    );

    // GET one → 200
    let resp = app
        .clone()
        .oneshot(get(&format!("/v1/queries/{id}"))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // UPDATE → 200
    let update = json!({ "name": "renamed", "sql": "SELECT 2" });
    let resp = app
        .clone()
        .oneshot(json_req("PUT", &format!("/v1/queries/{id}"), &update)?)
        .await?;
    assert_eq!(resp.status(), StatusCode::OK, "update should 200");

    // DELETE → 204
    let resp = app
        .clone()
        .oneshot(delete_req(&format!("/v1/queries/{id}"))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "delete should 204");

    // GET after delete → 404
    let resp = app.oneshot(get(&format!("/v1/queries/{id}"))?).await?;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "deleted query is gone"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn saved_query_create_with_invalid_sql_returns_400() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    // Non-read statement: the single-SELECT gate rejects it on write.
    let bad = json!({ "name": "bad", "sql": "DROP TABLE t" });
    let resp = app.oneshot(json_req("POST", "/v1/queries", &bad)?).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn saved_query_update_with_invalid_sql_returns_400() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();
    let app = app(db, tenant);

    let resp = app
        .clone()
        .oneshot(json_req("POST", "/v1/queries", &create_body())?)
        .await?;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await?;
    let id = created["id"].as_str().unwrap_or_default().to_owned();

    // Multiple statements are rejected by the gate on update.
    let bad = json!({ "sql": "SELECT 1; SELECT 2" });
    let resp = app
        .oneshot(json_req("PUT", &format!("/v1/queries/{id}"), &bad)?)
        .await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn get_unknown_saved_query_returns_404() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    let resp = app
        .oneshot(get(&format!("/v1/queries/{}", Uuid::now_v7()))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn update_unknown_saved_query_returns_404() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    let resp = app
        .oneshot(json_req(
            "PUT",
            &format!("/v1/queries/{}", Uuid::now_v7()),
            &json!({ "name": "x" }),
        )?)
        .await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn delete_unknown_saved_query_returns_404() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    let resp = app
        .oneshot(delete_req(&format!("/v1/queries/{}", Uuid::now_v7()))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn run_unknown_saved_query_returns_404() -> TestResult {
    // The run handler resolves the saved query (tenant-scoped) before touching
    // ClickHouse, so an unknown id is a clean 404 that never reaches `dead_ch`.
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());
    let resp = app
        .oneshot(json_req(
            "POST",
            &format!("/v1/queries/{}/run", Uuid::now_v7()),
            &json!({}),
        )?)
        .await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn saved_query_is_tenant_scoped() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant_a = Uuid::now_v7();
    let app_a = app(db.clone(), tenant_a);
    let resp = app_a
        .clone()
        .oneshot(json_req("POST", "/v1/queries", &create_body())?)
        .await?;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await?;
    let id = created["id"].as_str().unwrap_or_default().to_owned();

    let app_b = app(db, Uuid::now_v7());

    let resp = app_b.clone().oneshot(get("/v1/queries")?).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let list = body_json(resp).await?;
    let items = list["items"]
        .as_array()
        .unwrap_or_else(|| panic!("list payload has no items array: {list}"));
    assert!(
        items.iter().all(|i| i["id"].as_str() != Some(id.as_str())),
        "tenant B's list must not contain tenant A's saved query: {list}"
    );

    let cross_tenant_requests = [
        get(&format!("/v1/queries/{id}"))?,
        json_req(
            "PUT",
            &format!("/v1/queries/{id}"),
            &json!({ "name": "hijacked" }),
        )?,
        delete_req(&format!("/v1/queries/{id}"))?,
        json_req("POST", &format!("/v1/queries/{id}/run"), &json!({}))?,
    ];
    for req in cross_tenant_requests {
        let label = format!("{} {}", req.method(), req.uri());
        let resp = app_b.clone().oneshot(req).await?;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "tenant B must not reach tenant A's saved query via {label}"
        );
    }

    let resp = app_a.oneshot(get(&format!("/v1/queries/{id}"))?).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await?;
    assert_eq!(
        body["name"],
        create_body()["name"],
        "tenant A's saved query must be unchanged after tenant B's attempts"
    );
    Ok(())
}

// ── Handlers that reach ClickHouse / Identity (5xx by design) ────

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn metric_results_answers_200_with_generic_error_views_when_clickhouse_is_down() -> TestResult
{
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits"], &["repository"]).await?;
    let identity = spawn_identity(&[VISIBLE_PERSON]).await?;
    let result: anyhow::Result<()> = async {
        // A visible person id, not an email: since the identity cutover an email
        // is refused by validation, which would give a 4xx before the request
        // ever reached the ClickHouse failure this test is about. Every view
        // kind is requested so each query shape — batched, single, and the
        // group-ranking pre-pass — answers its own error slot.
        let app = app_with_identity(db.clone(), fixture.tenant_id, identity);
        let views = json!([
            {"view": "period"},
            {"view": "peer"},
            {"view": "timeseries", "bucket": "day"},
            {"view": "breakdown", "dimensions": ["repository"]},
            {"view": "rollup", "dimensions": ["repository"],
             "group_limit": {"count": 3, "include_remainder": true}}
        ]);
        let resp = app
            .oneshot(json_req(
                "POST",
                "/v1/metric-results",
                &json!({
                    "entity": {"type": "person", "ids": [VISIBLE_PERSON.to_string()]},
                    "period": {"from": "2026-07-01", "to": "2026-07-28"},
                    "metrics": [{
                        "metric_key": "git.commits",
                        "views": views
                    }]
                }),
            )?)
            .await?;
        anyhow::ensure!(
            resp.status() == StatusCode::OK,
            "a ClickHouse outage must not 500 the request, got {}",
            resp.status()
        );

        let body = body_json(resp).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let metric = &body["metrics"][0];
        anyhow::ensure!(metric["metric_key"] == "git.commits");
        anyhow::ensure!(
            metric["drilldown"].is_object(),
            "drilldown capabilities still load alongside the failed views"
        );
        let views = metric["views"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("views must be an array"))?;
        anyhow::ensure!(views.len() == 5, "every requested view keeps its slot");
        for view in views {
            anyhow::ensure!(view["view"] == "error", "each failed view answers in place");
            anyhow::ensure!(view["code"] == "QUERY_FAILED");
            let message = view["message"].as_str().unwrap_or_default();
            anyhow::ensure!(
                !message.contains("127.0.0.1") && !message.contains("error sending request"),
                "a non-admin must not see transport detail, got: {message}"
            );
        }
        Ok(())
    }
    .await;
    fixture.delete(&db).await?;
    result.map_err(Into::into)
}

/// Creates the identity relations a person-entity read joins, and maps one
/// e-mail to one person. Both must exist or the read fails outright — dbt owns
/// them in a deployment, so a live test has to stand them up itself.
async fn seed_identity_relations(
    ch: &insight_clickhouse::Client,
    email: &str,
    person_id: Uuid,
) -> anyhow::Result<()> {
    ch.query("CREATE DATABASE IF NOT EXISTS identity")
        .execute()
        .await?;
    ch.query(
        "CREATE TABLE IF NOT EXISTS identity.person_map (email String, person_id UUID) \
         ENGINE = MergeTree ORDER BY email",
    )
    .execute()
    .await?;
    ch.query(
        "CREATE TABLE IF NOT EXISTS identity.account_assignment \
         (source_type String, source_id UUID, account_id String, person_id UUID, \
          created_at DateTime64(6)) \
         ENGINE = MergeTree ORDER BY (source_type, source_id, account_id)",
    )
    .execute()
    .await?;
    ch.query(&format!(
        "INSERT INTO identity.person_map VALUES ('{email}', '{person_id}')"
    ))
    .execute()
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB + ClickHouse (INTEGRATION_TESTS_MARIADB_URL / _CLICKHOUSE_URL)"]
async fn metric_results_mixes_data_and_error_views_in_one_response() -> TestResult {
    // The isolation contract end-to-end on real backends: the observation
    // relation exists in ClickHouse, so period and timeseries answer with
    // data, while the peer view — whose cohort relation does not exist —
    // answers with its own error view in the same response.
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let Some(ch) = live_ch_or_skip() else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits"], &[]).await?;
    let suffix = fixture.tenant_id.simple().to_string();
    let relation = format!("insight.test_{suffix}_metric_observations");
    ch.query("CREATE DATABASE IF NOT EXISTS insight")
        .execute()
        .await?;
    // The serving contract this build reads: `entity_id` is the SOURCE identity
    // and the runtime resolves it through the identity relations while it
    // serves, so the fixture seeds an e-mail and a map row rather than a
    // pre-resolved person id.
    ch.query(&format!(
        "CREATE TABLE {relation} (tenant_id UUID, source_key String, entity_type String, \
         entity_id String, account_source_type String, account_source_id String, \
         account_id String, metric_date Date, measure_key String, observed_at DateTime64(3), \
         value Float64, subject_key Nullable(String), \
         dimensions Array(Tuple(key String, value String, label Nullable(String)))) \
         ENGINE = MergeTree ORDER BY (tenant_id, metric_date)"
    ))
    .execute()
    .await?;
    // Per-suffix so parallel runs never contend for one e-mail in the shared map.
    let person_email = format!("person-{suffix}@example.test");
    ch.query(&format!(
        "INSERT INTO {relation} SELECT '{tenant}', 'test_{suffix}', 'person', '{person_email}', \
         '', '', '', \
         toDate('2026-07-05') + number, 'value_count', now64(3), toFloat64(number + 1), NULL, [] \
         FROM numbers(3)",
        tenant = fixture.tenant_id,
    ))
    .execute()
    .await?;
    seed_identity_relations(&ch, &person_email, VISIBLE_PERSON).await?;

    let identity = spawn_identity(&[VISIBLE_PERSON]).await?;
    let result: anyhow::Result<()> = async {
        let app = app_with_identity_and_ch(db.clone(), fixture.tenant_id, identity, ch.clone());
        let resp = app
            .oneshot(json_req(
                "POST",
                "/v1/metric-results",
                &json!({
                    "entity": {"type": "person", "ids": [VISIBLE_PERSON.to_string()]},
                    "period": {"from": "2026-07-01", "to": "2026-07-28"},
                    "metrics": [{
                        "metric_key": "git.commits",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "peer"}
                        ]
                    }]
                }),
            )?)
            .await?;
        anyhow::ensure!(resp.status() == StatusCode::OK, "got {}", resp.status());

        let body = body_json(resp).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let views = &body["metrics"][0]["views"];
        anyhow::ensure!(
            views[0]["view"] == "period",
            "the period view must answer with data, got {}",
            views[0]
        );
        anyhow::ensure!(
            views[0]["values"][0]["value"] == 6.0,
            "the period value must sum the seeded observations, got {}",
            views[0]["values"][0]
        );
        anyhow::ensure!(
            views[1]["view"] == "timeseries",
            "the timeseries view must answer with data, got {}",
            views[1]
        );
        anyhow::ensure!(
            views[2]["view"] == "error" && views[2]["code"] == "SOURCE_RELATION_MISSING",
            "the peer view must fail alone on its missing cohort relation, got {}",
            views[2]
        );
        Ok(())
    }
    .await;
    let _ = ch
        .query(&format!("DROP TABLE IF EXISTS {relation}"))
        .execute()
        .await;
    fixture.delete(&db).await?;
    result.map_err(Into::into)
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn admins_see_the_underlying_error_when_clickhouse_is_down() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits"], &[]).await?;
    let identity = spawn_identity_with_roles(&[VISIBLE_PERSON], true).await?;
    let result: anyhow::Result<()> = async {
        let app = app_with_identity(db.clone(), fixture.tenant_id, identity);
        let resp = app
            .oneshot(json_req(
                "POST",
                "/v1/metric-results",
                &json!({
                    "entity": {"type": "person", "ids": [VISIBLE_PERSON.to_string()]},
                    "period": {"from": "2026-07-01", "to": "2026-07-28"},
                    "metrics": [{
                        "metric_key": "git.commits",
                        "views": [{"view": "period"}]
                    }]
                }),
            )?)
            .await?;
        anyhow::ensure!(resp.status() == StatusCode::OK);

        let body = body_json(resp).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let view = &body["metrics"][0]["views"][0];
        anyhow::ensure!(view["view"] == "error");
        let message = view["message"].as_str().unwrap_or_default();
        anyhow::ensure!(
            !message.is_empty() && !message.contains("could not be computed"),
            "an admin must see the underlying description, got: {message}"
        );
        Ok(())
    }
    .await;
    fixture.delete(&db).await?;
    result.map_err(Into::into)
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn metric_drilldown_validates_selection_before_clickhouse_error() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits"], &["repository"]).await?;
    let identity = spawn_identity(&[VISIBLE_PERSON]).await?;
    let result: anyhow::Result<()> = async {
        // A visible person: the gate admits, so what these assertions pin is
        // validation reaching the unreachable ClickHouse snapshot probe. A 403
        // would mean the gate denied a person it was told is visible.
        let app = app_with_identity(db.clone(), fixture.tenant_id, identity);
        let body = json!({
            "metric_key": "git.commits",
            "entity": {"type": "person", "id": VISIBLE_PERSON.to_string()},
            "period": {"from": "2026-07-01", "to": "2026-07-28"},
            "filters": [{"dimension": "repository", "values": ["org/repo"]}],
            "display_dimensions": ["repository"],
            "limit": 100
        });
        let resp = app
            .clone()
            .oneshot(json_req("POST", "/v1/metric-drilldown", &body)?)
            .await?;
        anyhow::ensure!(resp.status() == StatusCode::BAD_REQUEST);
        let resp = app
            .clone()
            .oneshot(json_req(
                "POST",
                "/v1/metric-drilldown/export",
                &drilldown_export_body(VISIBLE_PERSON),
            )?)
            .await?;
        anyhow::ensure!(resp.status() == StatusCode::BAD_REQUEST);
        // The pre-cutover email shape fails in the entity parse, which runs
        // ahead of the gate — so it stays a 400 rather than becoming a 403.
        let export = json!({
            "metric_key": "git.commits",
            "entity": {"type": "person", "id": "person@example.com"},
            "period": {"from": "2026-07-01", "to": "2026-07-28"},
            "filters": [],
            "display_dimensions": [],
            "format": "csv"
        });
        let resp = app
            .oneshot(json_req("POST", "/v1/metric-drilldown/export", &export)?)
            .await?;
        anyhow::ensure!(resp.status() == StatusCode::BAD_REQUEST);
        Ok(())
    }
    .await;
    fixture.delete(&db).await?;
    result.map_err(Into::into)
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn metric_drilldown_rejects_invalid_selection_without_clickhouse() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    // Only the reversed-period case carries a parseable person id and so reaches
    // the gate; the rest fail in the entity parse ahead of it. The identity has
    // to be real either way, or that one case would fail closed with a 500.
    let identity = spawn_identity(&[VISIBLE_PERSON]).await?;
    let app = app_with_identity(db, Uuid::now_v7(), identity);
    for body in [
        json!({
            "metric_key": "git.commits",
            "entity": {"type": "team", "id": "team"},
            "period": {"from": "2026-07-01", "to": "2026-07-28"},
            "limit": 100
        }),
        json!({
            "metric_key": "git.commits",
            "entity": {"type": "person", "id": ""},
            "period": {"from": "2026-07-01", "to": "2026-07-28"},
            "limit": 100
        }),
        json!({
            "metric_key": "git.commits",
            "entity": {"type": "person", "id": VISIBLE_PERSON.to_string()},
            "period": {"from": "2026-07-28", "to": "2026-07-01"},
            "limit": 100
        }),
        // The pre-cutover email shape and the nil UUID: entity.id is a
        // canonical person id here like on every other person-keyed route.
        json!({
            "metric_key": "git.commits",
            "entity": {"type": "person", "id": "person@example.com"},
            "period": {"from": "2026-07-01", "to": "2026-07-28"},
            "limit": 100
        }),
        json!({
            "metric_key": "git.commits",
            "entity": {"type": "person", "id": "00000000-0000-0000-0000-000000000000"},
            "period": {"from": "2026-07-01", "to": "2026-07-28"},
            "limit": 100
        }),
    ] {
        let resp = app
            .clone()
            .oneshot(json_req("POST", "/v1/metric-drilldown", &body)?)
            .await?;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "should reject: {body:?}"
        );
    }
    Ok(())
}

// ── Usage monitoring (#2573) ─────────────────────────────────────

/// The seeded `admin` role id, mirrored from identity's `roles_repo`.
const ADMIN_ROLE: Uuid = Uuid::from_u128(0xa4d1_1000_0000_4000_8000_0000_0000_0001);
const SOME_OTHER_ROLE: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0009);

/// Loopback identity serving `GET /v1/me` with the roles it is given — the only
/// endpoint the usage admin gate reads.
async fn spawn_identity_me(roles: &[Uuid]) -> Result<IdentityClient, Box<dyn std::error::Error>> {
    let roles: Vec<Value> = roles
        .iter()
        .map(|role_id| json!({"role_id": role_id.to_string()}))
        .collect();
    let roles = Arc::new(roles);

    let app = Router::new().route(
        "/v1/me",
        axum::routing::get(move || {
            let roles = Arc::clone(&roles);
            async move { axum::Json(json!({"roles": *roles})) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(IdentityClient::new(&format!("http://{addr}"))?)
}

fn app_with_usage_collection_off(db: DatabaseConnection, tenant: Uuid) -> Router {
    let Ok(identity) = IdentityClient::new("http://127.0.0.1:1") else {
        unreachable!("the static identity url builds a client")
    };
    let mut config = GearConfig::default();
    config.usage.enabled = false;
    let openapi = OpenApiRegistryImpl::new();
    let state = Arc::new(AppState {
        db,
        ch: dead_ch(),
        identity,
        anthropic: dead_anthropic(),
        ai_calls: Arc::new(tokio::sync::Semaphore::new(1)),
        config,
        external_links: crate::domain::external_links::ExternalSourceRegistry::default(),
    });
    let api = super::build_operations(Router::new(), &openapi)
        .layer(from_fn_with_state(tenant, inject_host_context))
        .layer(axum::Extension(state));
    Router::new().merge(api)
}

/// One SDK v2 beacon carrying a single page view.
fn beacon(path: &str) -> Value {
    json!({
        "meta": {},
        "records": [{
            "name": "page_view",
            "context_session_id": "s-1",
            "context_app_name": "insight-frontend",
            "context_app_version": "0.0.0",
            "data": {"path": path},
        }],
    })
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn usage_config_is_served_to_any_signed_in_caller() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());

    let resp = app.oneshot(get("/v1/usage/config")?).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await?["enabled"], json!(true));
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn usage_config_reports_an_instance_that_collects_nothing() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app_with_usage_collection_off(db, Uuid::now_v7());

    let resp = app.oneshot(get("/v1/usage/config")?).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        body_json(resp).await?["enabled"],
        json!(false),
        "the SPA decides whether to start the SDK from this alone"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_beacon_is_accepted_even_when_the_event_store_is_unreachable() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7());

    let req = json_req("POST", "/v1/usage/events", &beacon("/portal/manage"))?;
    let resp = app.oneshot(req).await?;

    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "a tracking failure must never reach the reader — ClickHouse is dead here"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_beacon_is_accepted_and_dropped_when_collection_is_off() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app_with_usage_collection_off(db, Uuid::now_v7());

    let req = json_req("POST", "/v1/usage/events", &beacon("/portal/manage"))?;
    let resp = app.oneshot(req).await?;

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn the_usage_summary_is_refused_without_the_admin_role() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let identity = spawn_identity_me(&[SOME_OTHER_ROLE]).await?;
    let app = app_with_identity(db, Uuid::now_v7(), identity);

    let resp = app
        .oneshot(json_req("GET", "/v1/usage/summary", &json!({}))?)
        .await?;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "holding some other role is not administrative authority"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn the_usage_summary_fails_closed_when_the_role_check_cannot_be_made() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    // Identity is unreachable here: an admin surface must not open because the
    // service that knows who is an admin is down.
    let app = app(db, Uuid::now_v7());

    let resp = app
        .oneshot(json_req("GET", "/v1/usage/summary", &json!({}))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn the_usage_summary_admits_an_admin_and_reaches_the_store() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let identity = spawn_identity_me(&[ADMIN_ROLE]).await?;
    let app = app_with_identity(db, Uuid::now_v7(), identity);

    // ClickHouse is unreachable, so an admitted caller cannot reach a 200; a
    // 500 rather than a 403 is what shows the gate let them through.
    let resp = app
        .oneshot(json_req("GET", "/v1/usage/summary", &json!({}))?)
        .await?;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_malformed_day_is_refused_before_the_store_is_asked() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let identity = spawn_identity_me(&[ADMIN_ROLE]).await?;
    let app = app_with_identity(db, Uuid::now_v7(), identity);

    let req = json_req("GET", "/v1/usage/summary?since=not-a-date", &json!({}))?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}
