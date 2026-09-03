use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::{RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::config::McpConfig;
use crate::domain::query_gate::validate_mcp_sql;

const MAX_REQUEST_BODY_BYTES: usize = 128 * 1024;
const MAX_SQL_BYTES: usize = 64 * 1024;
const MAX_RESULT_BYTES: usize = 5 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(35);
const MAX_CONCURRENT_QUERIES: usize = 2;
const MAX_JWKS_BYTES: usize = 1024 * 1024;
const MCP_SCOPE: &str = "mcp:query";

#[derive(Debug, Deserialize, JsonSchema)]
struct QuerySqlRequest {
    /// One ClickHouse SELECT or WITH statement. Multiple CTEs and nested queries are allowed.
    sql: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ResultColumn {
    name: String,
    #[serde(rename = "type")]
    data_type: String,
}

#[derive(Debug, Deserialize)]
struct ClickHouseJsonResult {
    meta: Vec<ResultColumn>,
    data: Vec<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Serialize)]
struct QuerySqlResult {
    columns: Vec<ResultColumn>,
    rows: Vec<serde_json::Map<String, serde_json::Value>>,
    row_count: usize,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct McpAccessClaims {
    sub: String,
    tenant_id: String,
    roles: String,
    sub_type: String,
    sid: String,
    iss: String,
    aud: String,
    scope: String,
    iat: u64,
    exp: u64,
    jti: String,
}

#[derive(Clone)]
struct TokenVerifier {
    inner: Arc<TokenVerifierInner>,
}

struct TokenVerifierInner {
    issuer: String,
    audience: String,
    resource_metadata: String,
    jwks_url: String,
    client: reqwest::Client,
    jwks: RwLock<Option<JwkSet>>,
}

#[derive(Debug)]
enum AuthFailure {
    Unauthorized,
    InsufficientScope,
    Unavailable,
}

#[derive(Clone)]
struct SqlExplorer {
    client: insight_clickhouse::Client,
    query_slots: Arc<Semaphore>,
    tool_router: ToolRouter<Self>,
}

impl SqlExplorer {
    fn new(client: insight_clickhouse::Client) -> Self {
        Self {
            client,
            query_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_QUERIES)),
            tool_router: Self::tool_router(),
        }
    }

    async fn execute(&self, sql: &str) -> Result<QuerySqlResult, String> {
        let permit = self
            .query_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "the SQL explorer is busy; retry shortly".to_owned())?;

        let mut query = self
            .client
            .query(sql)
            .fetch_bytes("JSON")
            .map_err(|error| format!("ClickHouse rejected the query: {error}"))?;

        let fetch = async {
            let mut bytes = Vec::new();
            while let Some(chunk) = query
                .next()
                .await
                .map_err(|error| format!("ClickHouse query failed: {error}"))?
            {
                let next_len = bytes.len().saturating_add(chunk.len());
                if next_len > MAX_RESULT_BYTES {
                    return Err(format!(
                        "query result exceeded the {MAX_RESULT_BYTES}-byte response limit"
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice::<ClickHouseJsonResult>(&bytes)
                .map_err(|error| format!("ClickHouse returned invalid JSON: {error}"))
        };

        let result = tokio::time::timeout(FETCH_TIMEOUT, fetch)
            .await
            .map_err(|_| "query timed out".to_owned())??;
        drop(permit);

        let row_count = result.data.len();
        Ok(QuerySqlResult {
            columns: result.meta,
            rows: result.data,
            row_count,
            truncated: false,
        })
    }
}

#[tool_router]
impl SqlExplorer {
    #[tool(
        name = "query_sql",
        description = "Run one read-only ClickHouse SELECT/WITH query over bronze_*, staging, silver, gold, identity, and config data. Use system.databases, system.tables, and system.columns to discover schemas. Multiple CTEs, joins, unions, and nested subqueries are allowed; multiple top-level statements, query SETTINGS/FORMAT clauses, and external table functions are rejected."
    )]
    async fn query_sql(
        &self,
        Parameters(QuerySqlRequest { sql }): Parameters<QuerySqlRequest>,
    ) -> CallToolResult {
        if sql.len() > MAX_SQL_BYTES {
            return tool_error(format!(
                "SQL exceeds the {MAX_SQL_BYTES}-byte request limit"
            ));
        }
        if let Err(reason) = validate_mcp_sql(&sql) {
            return tool_error(reason);
        }

        let started = Instant::now();
        let query_hash = hex::encode(Sha256::digest(sql.as_bytes()));
        match self.execute(&sql).await {
            Ok(result) => {
                tracing::info!(
                    query_hash,
                    duration_ms = started.elapsed().as_millis(),
                    rows = result.row_count,
                    outcome = "success",
                    "MCP SQL query completed"
                );
                match serde_json::to_value(result) {
                    Ok(value) => CallToolResult::structured(value),
                    Err(error) => {
                        tracing::error!(query_hash, %error, "failed to serialize MCP SQL result");
                        tool_error("query result serialization failed")
                    }
                }
            }
            Err(message) => {
                tracing::warn!(
                    query_hash,
                    duration_ms = started.elapsed().as_millis(),
                    outcome = "error",
                    "MCP SQL query failed"
                );
                tool_error(message)
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SqlExplorer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("insight-sql-explorer", env!("CARGO_PKG_VERSION"))
                    .with_title("Insight SQL Explorer"),
            )
            .with_instructions(
                "Explore the ClickHouse warehouse through query_sql. Start with system.databases, then system.tables and system.columns. Query only bronze_*, staging, silver, identity, config, and the configured gold database. Prefer narrow SELECT lists and LIMIT clauses.",
            )
    }
}

pub async fn start(
    config: &McpConfig,
    clickhouse_url: &str,
    clickhouse_database: &str,
    cancellation_token: CancellationToken,
) -> anyhow::Result<()> {
    let bind_addr: SocketAddr = config.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    let client = insight_clickhouse::Client::new(
        insight_clickhouse::Config::new(clickhouse_url, clickhouse_database)
            .with_auth(
                &config.clickhouse_user,
                config.clickhouse_password.expose_secret(),
            )
            .without_query_timeout(),
    );
    let explorer = SqlExplorer::new(client);
    let service: StreamableHttpService<SqlExplorer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(explorer.clone()),
            Arc::default(),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(false)
                .with_json_response(true)
                .with_allowed_hosts(["localhost"])
                .with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES)
                .with_cancellation_token(cancellation_token.child_token()),
        );
    let auth = TokenVerifier::new(&config.public_url)?;
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(auth, authenticate));

    tracing::info!(%bind_addr, "MCP SQL explorer listening");
    tokio::spawn(async move {
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(cancellation_token.cancelled_owned())
            .await;
        if let Err(error) = result {
            tracing::error!(%error, "MCP SQL explorer stopped unexpectedly");
        }
    });

    Ok(())
}

pub fn validate_config(config: &McpConfig) -> anyhow::Result<()> {
    if !config.enabled {
        return Ok(());
    }
    config.bind_addr.parse::<SocketAddr>()?;
    validate_public_url(&config.public_url)?;
    if config.clickhouse_user.trim().is_empty() {
        anyhow::bail!("MCP ClickHouse user is empty");
    }
    if config.clickhouse_password.expose_secret().is_empty() {
        anyhow::bail!("MCP ClickHouse password is empty");
    }
    Ok(())
}

async fn authenticate(
    State(verifier): State<TokenVerifier>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let result = match bearer_token(&headers) {
        Some(token) => verifier.verify(token).await,
        None => Err(AuthFailure::Unauthorized),
    };
    match result {
        Ok(claims) => {
            tracing::debug!(
                subject = %claims.sub,
                tenant_id = %claims.tenant_id,
                session_id = %claims.sid,
                token_id = %claims.jti,
                "MCP request authenticated"
            );
            next.run(request).await
        }
        Err(AuthFailure::Unauthorized) => {
            verifier.challenge(StatusCode::UNAUTHORIZED, "invalid_token")
        }
        Err(AuthFailure::InsufficientScope) => {
            verifier.challenge(StatusCode::FORBIDDEN, "insufficient_scope")
        }
        Err(AuthFailure::Unavailable) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

impl TokenVerifier {
    fn new(public_url: &str) -> anyhow::Result<Self> {
        validate_public_url(public_url)?;
        let issuer = public_url.trim_end_matches('/').to_owned();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self {
            inner: Arc::new(TokenVerifierInner {
                audience: format!("{issuer}/mcp"),
                resource_metadata: format!("{issuer}/.well-known/oauth-protected-resource/mcp"),
                jwks_url: format!("{issuer}/.well-known/jwks.json"),
                issuer,
                client,
                jwks: RwLock::new(None),
            }),
        })
    }

    async fn verify(&self, token: &str) -> Result<McpAccessClaims, AuthFailure> {
        let header = decode_header(token).map_err(|_| AuthFailure::Unauthorized)?;
        if header.alg != Algorithm::ES256 {
            return Err(AuthFailure::Unauthorized);
        }
        let kid = header.kid.ok_or(AuthFailure::Unauthorized)?;
        let mut jwks = self.cached_jwks().await?;
        if jwks.find(&kid).is_none() {
            jwks = self.fetch_jwks().await?;
        }
        let jwk = jwks.find(&kid).ok_or(AuthFailure::Unauthorized)?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| AuthFailure::Unauthorized)?;

        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_issuer(&[self.inner.issuer.as_str()]);
        validation.set_audience(&[self.inner.audience.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        let claims = decode::<McpAccessClaims>(token, &key, &validation)
            .map_err(|_| AuthFailure::Unauthorized)?
            .claims;
        if claims.sub_type != "user"
            || claims
                .scope
                .split_whitespace()
                .all(|scope| scope != MCP_SCOPE)
            || claims.roles.split_whitespace().all(|role| role != "admin")
            || claims.iss != self.inner.issuer
            || claims.aud != self.inner.audience
            || claims.iat > claims.exp
        {
            return Err(AuthFailure::InsufficientScope);
        }
        Ok(claims)
    }

    async fn cached_jwks(&self) -> Result<JwkSet, AuthFailure> {
        if let Some(jwks) = self.inner.jwks.read().await.clone() {
            return Ok(jwks);
        }
        self.fetch_jwks().await
    }

    async fn fetch_jwks(&self) -> Result<JwkSet, AuthFailure> {
        let response = self
            .inner
            .client
            .get(&self.inner.jwks_url)
            .send()
            .await
            .map_err(|error| {
                tracing::warn!(%error, "could not fetch MCP JWKS");
                AuthFailure::Unavailable
            })?
            .error_for_status()
            .map_err(|error| {
                tracing::warn!(%error, "MCP JWKS endpoint returned an error");
                AuthFailure::Unavailable
            })?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_JWKS_BYTES as u64)
        {
            return Err(AuthFailure::Unavailable);
        }
        let bytes = response.bytes().await.map_err(|error| {
            tracing::warn!(%error, "could not read MCP JWKS response");
            AuthFailure::Unavailable
        })?;
        if bytes.len() > MAX_JWKS_BYTES {
            return Err(AuthFailure::Unavailable);
        }
        let jwks = serde_json::from_slice::<JwkSet>(&bytes).map_err(|error| {
            tracing::warn!(%error, "could not parse MCP JWKS response");
            AuthFailure::Unavailable
        })?;
        *self.inner.jwks.write().await = Some(jwks.clone());
        Ok(jwks)
    }

    fn challenge(&self, status: StatusCode, error: &str) -> Response {
        let value = format!(
            "Bearer resource_metadata=\"{}\", scope=\"{MCP_SCOPE}\", error=\"{error}\"",
            self.inner.resource_metadata
        );
        let mut response = status.into_response();
        if let Ok(value) = HeaderValue::from_str(&value) {
            response.headers_mut().insert(WWW_AUTHENTICATE, value);
        }
        response
    }
}

fn validate_public_url(public_url: &str) -> anyhow::Result<()> {
    let url = url::Url::parse(public_url)?;
    let local_http =
        url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    anyhow::ensure!(
        (url.scheme() == "https" || local_http)
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none(),
        "MCP public URL must be an HTTPS origin, or an HTTP localhost origin"
    );
    Ok(())
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty() && !token.chars().any(char::is_whitespace))
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}
