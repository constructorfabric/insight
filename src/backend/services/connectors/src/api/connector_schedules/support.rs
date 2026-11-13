use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use axum::response::Response;
use axum::{Extension, Json, Router};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use serde_json::{Value, json};
use toolkit::api::OpenApiRegistryImpl;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::AppState;
use crate::infra::identity::IdentityClient;

pub(super) type R = Result<(), Box<dyn std::error::Error>>;
pub(super) const CALLER: Uuid = Uuid::from_u128(1);

pub(super) struct Harness {
    pub app: Router,
    identity: tokio::task::JoinHandle<()>,
    pub state: Arc<AppState>,
    pub workflow: crate::infra::cluster_stub::Stub,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.identity.abort();
    }
}

pub(super) async fn harness(
    db: DatabaseConnection,
    ch_url: &str,
    ctx: SecurityContext,
    admin: bool,
) -> Result<Harness, Box<dyn std::error::Error>> {
    harness_for_cluster(
        db,
        ch_url,
        ctx,
        admin,
        crate::infra::cluster_stub::Scenario::Missing,
        "example-tracker",
    )
    .await
}

pub(super) async fn harness_for_cluster(
    db: DatabaseConnection,
    ch_url: &str,
    ctx: SecurityContext,
    admin: bool,
    scenario: crate::infra::cluster_stub::Scenario,
    connector: &str,
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
    let workflow = crate::infra::cluster_stub::Stub::start(scenario, connector, "main").await?;
    let mut state = state(db, ch_url, identity);
    Arc::get_mut(&mut state).ok_or("shared state")?.cluster = workflow.cluster.clone();
    let app = router(Arc::clone(&state), ctx);
    Ok(Harness {
        app,
        identity: identity_task,
        state,
        workflow,
    })
}

pub(super) fn state(
    db: DatabaseConnection,
    ch_url: &str,
    identity: IdentityClient,
) -> Arc<AppState> {
    Arc::new(AppState {
        db,
        ch: insight_clickhouse::Client::new(insight_clickhouse::Config::new(ch_url, "analytics")),
        identity,
        cluster: crate::infra::cluster::Cluster::disabled(),
        schedule_slots: tokio::sync::Semaphore::new(4),
    })
}

pub(super) fn router(state: Arc<AppState>, ctx: SecurityContext) -> Router {
    crate::api::build_operations(Router::new(), &OpenApiRegistryImpl::new())
        .layer(Extension(ctx))
        .layer(Extension(state))
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
    pub connector: String,
}

pub(super) fn configured_instance_record() -> ConfiguredInstanceRecord {
    ConfiguredInstanceRecord {
        connector: "example-tracker".to_owned(),
    }
}
