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

/// The live DB, or `None` when this run is not meant to have one.
///
/// A set-but-unreachable URL is an error, never a skip: swallowing it would
/// turn a broken database into a green suite.
async fn connect_or_skip() -> Result<Option<DatabaseConnection>, Box<dyn std::error::Error>> {
    let Ok(url) = std::env::var(ENV_VAR) else {
        eprintln!("skipping: {ENV_VAR} not set");
        return Ok(None);
    };
    let mut opts = ConnectOptions::new(url);
    opts.max_connections(4).sqlx_logging(false);

    Ok(Some(Database::connect(opts).await?))
}

fn enabled_config() -> GearConfig {
    GearConfig {
        ai_assist: AiAssistConfig {
            enabled: true,
            token_encryption_key: BASE64.encode([11_u8; KEY_BYTES]),
            api_base: "http://127.0.0.1:1".to_owned(),
            request_timeout_secs: 1,
            // Identity is unreachable in this harness, so an admin check
            // could only ever fail closed. The explain paths below are about
            // the key and the upstream; the gate has its own test.
            admin_only: false,
            ..AiAssistConfig::default()
        },
        ..GearConfig::default()
    }
}

fn admin_gated_config() -> GearConfig {
    GearConfig {
        ai_assist: AiAssistConfig {
            admin_only: true,
            ..enabled_config().ai_assist
        },
        ..GearConfig::default()
    }
}

fn stand_key_config() -> GearConfig {
    GearConfig {
        ai_assist: AiAssistConfig {
            api_key: "sk-ant-stand-key".to_owned(),
            ..enabled_config().ai_assist
        },
        ..GearConfig::default()
    }
}

fn app(db: DatabaseConnection, tenant: Uuid, config: GearConfig) -> Router {
    app_as(db, tenant, CALLER, config)
}

fn app_as(db: DatabaseConnection, tenant: Uuid, person: Uuid, config: GearConfig) -> Router {
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
        external_links: crate::domain::external_links::ExternalSourceRegistry::default(),
    });

    let openapi = OpenApiRegistryImpl::new();
    let api = crate::api::build_operations(Router::new(), &openapi)
        .layer(from_fn_with_state((tenant, person), inject_context))
        .layer(axum::Extension(state));
    Router::new().merge(api)
}

async fn inject_context(
    axum::extract::State((tenant, person)): axum::extract::State<(Uuid, Uuid)>,
    mut req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let Ok(ctx) = SecurityContext::builder()
        .subject_id(person)
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

mod context;
mod credentials;
mod explain;
mod gating;
mod settings;
