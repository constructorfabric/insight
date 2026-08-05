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

/// Build a full `AppState` against the live DB.
fn build_state(db: DatabaseConnection, identity: IdentityClient) -> AppState {
    AppState {
        db,
        ch: dead_ch(),
        identity,
        config: GearConfig::default(),
    }
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
    let openapi = OpenApiRegistryImpl::new();
    let state = Arc::new(build_state(db, identity));
    let api = super::build_operations(Router::new(), &openapi)
        .layer(from_fn_with_state(tenant, inject_host_context))
        .layer(axum::Extension(state));
    Router::new().merge(api)
}

/// Loopback identity serving `POST /v1/visible-persons` — answers with the
/// intersection of the requested person ids and `visible`.
async fn spawn_identity(visible: &[Uuid]) -> Result<IdentityClient, Box<dyn std::error::Error>> {
    let visible = Arc::new(
        visible
            .iter()
            .copied()
            .collect::<std::collections::HashSet<Uuid>>(),
    );

    let app = Router::new().route(
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
    // ClickHouse is unreachable here, so an admitted request cannot reach a
    // 200; what this pins is that the gate did not reject it. Asserting the
    // exact 500 (not merely `!= 403`) is deliberate: a 400 from validation
    // would satisfy a not-forbidden assertion and hide a request that never
    // reached the gate at all.
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
        StatusCode::INTERNAL_SERVER_ERROR,
        "a visible person must pass the gate and fail only on the unreachable \
         ClickHouse; a 400 would mean the request never got that far"
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
async fn metric_results_loads_drilldown_capabilities_before_clickhouse_error() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits"], &[]).await?;
    let identity = spawn_identity(&[VISIBLE_PERSON]).await?;
    let result: anyhow::Result<()> = async {
        // A visible person id, not an email: since the identity cutover an email
        // is refused by validation, which would give a 4xx before the request
        // ever reached the drilldown-capability load this test is about.
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
        anyhow::ensure!(resp.status().is_server_error());
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
    let result: anyhow::Result<()> = async {
        let app = app(db.clone(), fixture.tenant_id);
        let body = json!({
            "metric_key": "git.commits",
            "entity": {"type": "person", "id": "019e2830-0000-7000-8000-000000000001"},
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
    let app = app(db, Uuid::now_v7());
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
            "entity": {"type": "person", "id": "019e2830-0000-7000-8000-000000000001"},
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
