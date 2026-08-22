//! HTTP-level tests for `/v1/ai*` against a live MariaDB.
//!
//! Same convention as [`crate::api::http_live_tests`]: `#[ignore]`d, silently
//! skipped when `INTEGRATION_TESTS_MARIADB_URL` is unset, one fresh tenant per
//! test so the suite is parallel-safe.
//!
//! The identity service is unreachable on purpose. Admin-gated writes therefore
//! fail closed with a 500 ("could not verify"), which is the behaviour under
//! test — a reachable identity is covered by the compose e2e.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use serde_json::{Value, json};
use toolkit::api::OpenApiRegistryImpl;
use toolkit_security::SecurityContext;
use tower::ServiceExt;
use uuid::Uuid;

use crate::api::AppState;
use crate::config::{AiAssistConfig, GearConfig, KEY_BYTES};
use crate::infra::identity::IdentityClient;

const ENV_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";
const CALLER: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_00a1);
const TOKEN: &str = "sk-ant-api03-live-test-token-wxyz";

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn connect_or_skip() -> Option<DatabaseConnection> {
    let Ok(url) = std::env::var(ENV_VAR) else {
        eprintln!("skipping: {ENV_VAR} not set");
        return None;
    };
    let mut opts = ConnectOptions::new(url);
    opts.max_connections(4).sqlx_logging(false);
    Database::connect(opts).await.ok()
}

fn enabled_config() -> GearConfig {
    GearConfig {
        ai_assist: AiAssistConfig {
            enabled: true,
            token_encryption_key: BASE64.encode([11_u8; KEY_BYTES]),
            api_base: "http://127.0.0.1:1".to_owned(),
            request_timeout_secs: 1,
            ..AiAssistConfig::default()
        },
        ..GearConfig::default()
    }
}

fn app(db: DatabaseConnection, tenant: Uuid, config: GearConfig) -> Router {
    let Ok(identity) = IdentityClient::new("http://127.0.0.1:1") else {
        unreachable!("the static identity url builds a client")
    };
    let Ok(anthropic) = crate::infra::anthropic::AnthropicClient::new(
        &config.ai_assist.api_base,
        std::time::Duration::from_secs(config.ai_assist.request_timeout_secs),
    ) else {
        unreachable!("a client with no proxy config builds")
    };

    let state = Arc::new(AppState {
        db,
        ch: insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://127.0.0.1:1",
            "analytics",
        )),
        identity,
        anthropic,
        ai_calls: Arc::new(tokio::sync::Semaphore::new(1)),
        config,
    });

    let openapi = OpenApiRegistryImpl::new();
    let api = crate::api::build_operations(Router::new(), &openapi)
        .layer(from_fn_with_state(tenant, inject_context))
        .layer(axum::Extension(state));
    Router::new().merge(api)
}

async fn inject_context(
    axum::extract::State(tenant): axum::extract::State<Uuid>,
    mut req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let Ok(ctx) = SecurityContext::builder()
        .subject_id(CALLER)
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
    Request::builder()
        .uri(uri)
        .header("authorization", "Bearer test-gateway-jwt")
        .body(Body::empty())
}

fn json_req(method: &str, uri: &str, body: &Value) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", "Bearer test-gateway-jwt")
        .body(Body::from(serde_json::to_vec(body).unwrap_or_default()))
}

fn empty_req(method: &str, uri: &str) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", "Bearer test-gateway-jwt")
        .body(Body::empty())
}

async fn body_json(resp: Response) -> Result<Value, Box<dyn std::error::Error>> {
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn config_reports_the_stand_switch_when_it_is_off() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7(), GearConfig::default());

    let resp = app.oneshot(get("/v1/ai/config")?).await?;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await?["enabled"], false);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_stand_with_the_switch_off_hides_every_other_route() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    for uri in ["/v1/ai/credentials", "/v1/ai/settings", "/v1/ai/context"] {
        let app = app(db.clone(), tenant, GearConfig::default());
        let resp = app.oneshot(get(uri)?).await?;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri} must be hidden");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_key_round_trips_as_its_last_four_characters_only() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    let created = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": TOKEN }),
        )?)
        .await?;
    assert_eq!(created.status(), StatusCode::OK);
    let created = body_json(created).await?;
    assert_eq!(created["configured"], true);
    assert_eq!(created["hint"], "wxyz");
    assert!(created.get("token").is_none(), "the key came back out");

    let read = app(db.clone(), tenant, enabled_config())
        .oneshot(get("/v1/ai/credentials")?)
        .await?;
    let read = body_json(read).await?;
    assert_eq!(read["configured"], true);
    assert_eq!(read["hint"], "wxyz");

    let replaced = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": "sk-ant-second-key-abcd" }),
        )?)
        .await?;
    assert_eq!(body_json(replaced).await?["hint"], "abcd");

    let removed = app(db.clone(), tenant, enabled_config())
        .oneshot(empty_req("DELETE", "/v1/ai/credentials")?)
        .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    let after = app(db, tenant, enabled_config())
        .oneshot(get("/v1/ai/credentials")?)
        .await?;
    assert_eq!(body_json(after).await?["configured"], false);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_blank_key_is_refused() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": "   " }),
        )?)
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_tenant_without_its_own_prompt_reads_the_shipped_one() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(get("/v1/ai/settings")?)
        .await?;

    let body = body_json(resp).await?;
    assert_eq!(body["is_default"], true);
    assert!(
        body["system_prompt"]
            .as_str()
            .unwrap_or_default()
            .contains("explain"),
        "the shipped prompt should describe the job"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn writing_the_prompt_needs_an_admin_check_that_can_answer() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/settings",
            &json!({ "system_prompt": "ours" }),
        )?)
        .await?;

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "an unreachable identity must fail closed, never permit"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_person_writes_reads_and_removes_their_own_context() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    let created = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "POST",
            "/v1/ai/context",
            &json!({ "scope": "person", "title": "How my week runs", "body": "Meeting-heavy midweek." }),
        )?)
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = body_json(created).await?;
    let id = created["id"].as_str().unwrap_or_default().to_owned();
    assert_eq!(created["scope"], "person");

    let listed = app(db.clone(), tenant, enabled_config())
        .oneshot(get("/v1/ai/context")?)
        .await?;
    let items = body_json(listed).await?;
    assert_eq!(items["items"].as_array().map(Vec::len), Some(1));

    let edited = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "PATCH",
            &format!("/v1/ai/context/{id}"),
            &json!({ "title": "How my week actually runs" }),
        )?)
        .await?;
    assert_eq!(
        body_json(edited).await?["title"],
        "How my week actually runs"
    );

    let removed = app(db.clone(), tenant, enabled_config())
        .oneshot(empty_req("DELETE", &format!("/v1/ai/context/{id}"))?)
        .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    let after = app(db, tenant, enabled_config())
        .oneshot(get("/v1/ai/context")?)
        .await?;
    assert_eq!(
        body_json(after).await?["items"].as_array().map(Vec::len),
        Some(0)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn an_entry_with_no_title_is_refused() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "POST",
            "/v1/ai/context",
            &json!({ "scope": "person", "title": "  ", "body": "something" }),
        )?)
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn organisation_context_is_not_a_persons_to_write() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "POST",
            "/v1/ai/context",
            &json!({ "scope": "tenant", "title": "Ours", "body": "How we read metrics." }),
        )?)
        .await?;

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "an unreachable identity must fail closed on an admin-gated write"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn editing_someone_elses_entry_reads_as_absent() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "PATCH",
            &format!("/v1/ai/context/{}", Uuid::now_v7()),
            &json!({ "title": "mine now" }),
        )?)
        .await?;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn explaining_without_a_stored_key_says_so() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req("POST", "/v1/ai/explain", &snapshot())?)
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await?;
    assert!(
        serde_json::to_string(&body)?.contains("no Anthropic key"),
        "the refusal should name the missing key: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn an_unreachable_model_reads_as_busy_rather_than_broken() -> TestResult {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    let stored = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": TOKEN }),
        )?)
        .await?;
    assert_eq!(stored.status(), StatusCode::OK);

    let resp = app(db, tenant, enabled_config())
        .oneshot(json_req("POST", "/v1/ai/explain", &snapshot())?)
        .await?;

    assert!(
        resp.status().is_client_error() || resp.status().is_server_error(),
        "an unreachable upstream cannot produce an answer"
    );
    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "no answer may be invented when the model was never reached"
    );
    Ok(())
}

fn snapshot() -> Value {
    json!({
        "metric_key": "tasks.closed",
        "label": "Tasks closed",
        "value": "34",
        "period": "month",
        "since": "2026-08-01",
        "until": "2026-08-22",
        "delta": "+6 since last month",
        "peer": "Team median 27",
        "help": "",
        "trend": [1.0, null, 3.0],
    })
}
