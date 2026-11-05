use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use axum::response::Response;
use axum::{Extension, Json, Router};
use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use serde_json::{Value, json};
use toolkit::api::OpenApiRegistryImpl;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::AppState;
use crate::config::{ConnectorSchedulesConfig, GearConfig};
use crate::infra::identity::IdentityClient;

pub(super) type R = Result<(), Box<dyn std::error::Error>>;
pub(super) const CALLER: Uuid = Uuid::from_u128(1);
pub(super) const URI: &str = "/v1/connectors/example-tracker/main/schedule";

pub(super) struct Harness {
    pub app: Router,
    identity: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.identity.abort();
    }
}

pub(super) async fn harness(
    db: DatabaseConnection,
    ch_url: &str,
    enabled: bool,
    ctx: SecurityContext,
    admin: bool,
) -> Result<Harness, Box<dyn std::error::Error>> {
    let roles = if admin {
        json!([{"role_id": "a4d11000-0000-4000-8000-000000000001"}])
    } else {
        json!([])
    };
    let identity_app = Router::new().route(
        "/v1/me",
        axum::routing::get(move || async move { Json(json!({"roles": roles})) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let identity = IdentityClient::new(&format!("http://{}", listener.local_addr()?))?;
    let identity_task = tokio::spawn(async move {
        let _ = axum::serve(listener, identity_app).await;
    });
    let app = app(db, ch_url, enabled, ctx, identity)?;
    Ok(Harness {
        app,
        identity: identity_task,
    })
}

pub(super) fn app(
    db: DatabaseConnection,
    ch_url: &str,
    enabled: bool,
    ctx: SecurityContext,
    identity: IdentityClient,
) -> Result<Router, Box<dyn std::error::Error>> {
    let config = GearConfig {
        connector_schedules: ConnectorSchedulesConfig {
            enabled,
            reconcile_schedules: vec!["*/15 * * * *".to_owned()],
            ..ConnectorSchedulesConfig::default()
        },
        ..GearConfig::default()
    };
    let state = Arc::new(AppState {
        db,
        ch: insight_clickhouse::Client::new(insight_clickhouse::Config::new(ch_url, "analytics")),
        identity,
        anthropic: crate::infra::anthropic::AnthropicClient::new(
            "http://127.0.0.1:1",
            std::time::Duration::from_secs(1),
        )?,
        ai_calls: Arc::new(tokio::sync::Semaphore::new(1)),
        report_generations: Arc::new(tokio::sync::Semaphore::new(1)),
        report_artifacts: Arc::new(tokio::sync::Semaphore::new(1)),
        config,
        external_links: crate::domain::external_links::ExternalSourceRegistry::default(),
    });
    let app = crate::api::build_operations(Router::new(), &OpenApiRegistryImpl::new())
        .layer(Extension(ctx))
        .layer(Extension(state));
    Ok(app)
}

pub(super) fn context(
    subject: &str,
    scopes: &[&str],
) -> Result<SecurityContext, Box<dyn std::error::Error>> {
    SecurityContext::builder()
        .subject_id(CALLER)
        .subject_type(subject)
        .subject_tenant_id(Uuid::from_u128(2))
        .token_scopes(scopes.iter().map(|s| (*s).to_owned()).collect())
        .build()
        .map_err(|e| format!("security context: {e:?}").into())
}

pub(super) fn request(
    method: &str,
    uri: &str,
    revision: Option<&str>,
    body: &Value,
) -> Result<Request<Body>, axum::http::Error> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", "Bearer synthetic-token")
        .header("content-type", "application/json");
    if let Some(revision) = revision {
        builder = builder.header("If-Match", revision);
    }
    builder.body(Body::from(body.to_string()))
}

pub(super) async fn body(response: Response) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(
        &to_bytes(response.into_body(), 1024 * 1024).await?,
    )?)
}

#[derive(Debug, Serialize, clickhouse::Row)]
pub(super) struct ConfiguredInstanceRecord {
    pub schedules: Vec<String>,
    pub schedule_timezone: String,
    pub schedule_origin: String,
    pub schedule_suspended: bool,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pub pass_completed_at: DateTime<Utc>,
}

pub(super) fn configured_instance_record() -> ConfiguredInstanceRecord {
    ConfiguredInstanceRecord {
        schedules: vec!["0 3 * * *".to_owned()],
        schedule_timezone: "UTC".to_owned(),
        schedule_origin: "api".to_owned(),
        schedule_suspended: true,
        pass_completed_at: DateTime::UNIX_EPOCH,
    }
}
