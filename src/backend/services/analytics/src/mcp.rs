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
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::config::McpConfig;
use crate::domain::query_gate::validate_mcp_sql;

const MAX_REQUEST_BODY_BYTES: usize = 128 * 1024;
const MAX_SQL_BYTES: usize = 64 * 1024;
const MAX_RESULT_BYTES: usize = 5 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(35);
const MAX_JWKS_BYTES: usize = 1024 * 1024;
const JWKS_REFRESH_COOLDOWN: Duration = Duration::from_secs(5);
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
    #[serde(rename = "sub")]
    _sub: String,
    #[serde(rename = "tenant_id")]
    _tenant_id: String,
    roles: String,
    sub_type: String,
    #[serde(rename = "sid")]
    _sid: String,
    iss: String,
    aud: String,
    scope: String,
    iat: u64,
    exp: u64,
    #[serde(rename = "jti")]
    _jti: String,
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
    jwks_refresh: Mutex<JwksRefresh>,
}

#[derive(Default)]
struct JwksRefresh {
    last_attempt: Option<Instant>,
}

#[derive(Debug)]
enum AuthFailure {
    Unauthorized,
    InsufficientScope,
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
enum QueryFailure {
    #[error("query capacity is exhausted")]
    Busy,
    #[error("query result exceeded the response limit")]
    ResultTooLarge,
    #[error("query timed out")]
    Timeout,
    #[error("ClickHouse query failed: {0}")]
    ClickHouse(#[from] clickhouse::error::Error),
    #[error("ClickHouse returned an invalid JSON response: {0}")]
    InvalidResponse(#[from] serde_json::Error),
    #[error("query result processing task failed: {0}")]
    ProcessingTask(#[from] tokio::task::JoinError),
}

impl QueryFailure {
    fn public_message(&self) -> &'static str {
        match self {
            Self::Busy => "the SQL explorer is busy; retry shortly",
            Self::ResultTooLarge => "query result exceeded the response limit",
            Self::Timeout => "query timed out",
            Self::ClickHouse(_) | Self::InvalidResponse(_) | Self::ProcessingTask(_) => {
                "query execution failed"
            }
        }
    }
}

#[derive(Clone)]
struct SqlExplorer {
    client: insight_clickhouse::Client,
    query_slots: Arc<Semaphore>,
    tool_router: ToolRouter<Self>,
}

impl SqlExplorer {
    fn new(client: insight_clickhouse::Client, max_concurrent_queries: usize) -> Self {
        Self {
            client,
            query_slots: Arc::new(Semaphore::new(max_concurrent_queries)),
            tool_router: Self::tool_router(),
        }
    }

    async fn execute(&self, sql: &str) -> Result<QuerySqlResult, QueryFailure> {
        let permit = self
            .query_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| QueryFailure::Busy)?;

        let mut query = self.client.query(sql).fetch_bytes("JSON")?;

        let fetch = async {
            let mut bytes = Vec::new();
            while let Some(chunk) = query.next().await? {
                let next_len = bytes.len().saturating_add(chunk.len());
                if next_len > MAX_RESULT_BYTES {
                    return Err(QueryFailure::ResultTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok::<_, QueryFailure>(bytes)
        };

        let bytes = tokio::time::timeout(FETCH_TIMEOUT, fetch)
            .await
            .map_err(|_| QueryFailure::Timeout)??;
        // INVARIANT: query capacity also bounds CPU-heavy result decoding.
        let result = tokio::task::spawn_blocking(move || {
            serde_json::from_slice::<ClickHouseJsonResult>(&bytes)
        })
        .await??;
        drop(permit);

        let row_count = result.data.len();
        Ok(QuerySqlResult {
            columns: result.meta,
            rows: result.data,
            row_count,
            truncated: false,
        })
    }

    async fn run_query_sql(&self, sql: String) -> CallToolResult {
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
                match tokio::task::spawn_blocking(move || serde_json::to_value(result)).await {
                    Ok(Ok(value)) => CallToolResult::structured(value),
                    Ok(Err(error)) => {
                        tracing::error!(query_hash, %error, "failed to serialize MCP SQL result");
                        tool_error("query result serialization failed")
                    }
                    Err(error) => {
                        tracing::error!(query_hash, %error, "MCP result serialization task failed");
                        tool_error("query result serialization failed")
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    query_hash,
                    duration_ms = started.elapsed().as_millis(),
                    error = %error,
                    outcome = "error",
                    "MCP SQL query failed"
                );
                tool_error(error.public_message())
            }
        }
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
        self.run_query_sql(sql).await
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
    let explorer = SqlExplorer::new(client, config.max_concurrent_queries);
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
    let auth = TokenVerifier::new(&config.public_url, config.allow_insecure_private_network)?;
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
    validate_public_url(&config.public_url, config.allow_insecure_private_network)?;
    if config.max_concurrent_queries == 0 {
        anyhow::bail!("MCP max concurrent queries must be greater than zero");
    }
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
        Ok(_) => next.run(request).await,
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
    fn new(public_url: &str, allow_insecure_private_network: bool) -> anyhow::Result<Self> {
        validate_public_url(public_url, allow_insecure_private_network)?;
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
                jwks_refresh: Mutex::new(JwksRefresh::default()),
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
            jwks = self.refresh_jwks(Some(&kid)).await?;
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
        self.refresh_jwks(None).await
    }

    async fn refresh_jwks(&self, expected_kid: Option<&str>) -> Result<JwkSet, AuthFailure> {
        let mut refresh = self.inner.jwks_refresh.lock().await;
        let cached = self.inner.jwks.read().await.clone();
        let cache_satisfies = match expected_kid {
            Some(kid) => cached.as_ref().is_some_and(|jwks| jwks.find(kid).is_some()),
            None => cached.is_some(),
        };
        if cache_satisfies {
            return cached.ok_or(AuthFailure::Unavailable);
        }
        if refresh
            .last_attempt
            .is_some_and(|attempt| attempt.elapsed() < JWKS_REFRESH_COOLDOWN)
        {
            return cached.ok_or(AuthFailure::Unavailable);
        }

        refresh.last_attempt = Some(Instant::now());
        // INVARIANT: holding the gate across fetch permits only one JWKS refresh.
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

fn validate_public_url(
    public_url: &str,
    allow_insecure_private_network: bool,
) -> anyhow::Result<()> {
    let url = url::Url::parse(public_url)?;
    let local_http =
        url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    let private_http = url.scheme() == "http"
        && allow_insecure_private_network
        && url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| match address {
                std::net::IpAddr::V4(address) => address.is_private(),
                std::net::IpAddr::V6(address) => (address.segments()[0] & 0xfe00) == 0xfc00,
            });
    anyhow::ensure!(
        (url.scheme() == "https" || local_http || private_http)
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none(),
        "MCP public URL must be an HTTPS origin or an allowed local HTTP origin"
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::Json;
    use axum::body::Body;
    use axum::http::{HeaderValue, Request, Response, StatusCode};
    use axum::routing::{any, get};
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use p256::SecretKey;
    use p256::elliptic_curve::Generate as _;
    use p256::elliptic_curve::sec1::ToSec1Point as _;
    use p256::pkcs8::{EncodePrivateKey as _, LineEnding};
    use secrecy::SecretString;
    use serde_json::{Value, json};
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt as _;

    use super::{
        MAX_RESULT_BYTES, MAX_SQL_BYTES, McpConfig, QueryFailure, QuerySqlRequest, SqlExplorer,
        TokenVerifier, bearer_token, start, validate_config, validate_public_url,
    };

    struct SigningMaterial {
        key: EncodingKey,
        jwks: Value,
    }

    fn signing_material() -> anyhow::Result<SigningMaterial> {
        let secret = SecretKey::generate();
        let pem = secret.to_pkcs8_pem(LineEnding::LF)?;
        let key = EncodingKey::from_ec_pem(pem.as_bytes())?;
        let point = secret.public_key().to_sec1_point(false);
        let x = point
            .x()
            .ok_or_else(|| anyhow::anyhow!("public key has no x coordinate"))?;
        let y = point
            .y()
            .ok_or_else(|| anyhow::anyhow!("public key has no y coordinate"))?;

        Ok(SigningMaterial {
            key,
            jwks: json!({
                "keys": [{
                    "kty": "EC",
                    "crv": "P-256",
                    "use": "sig",
                    "alg": "ES256",
                    "kid": "test-key",
                    "x": B64.encode(x),
                    "y": B64.encode(y),
                }]
            }),
        })
    }

    fn claims(issuer: &str) -> anyhow::Result<Value> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        Ok(json!({
            "sub": "test-user",
            "tenant_id": "test-tenant",
            "roles": "user admin",
            "sub_type": "user",
            "sid": "test-session",
            "iss": issuer,
            "aud": format!("{issuer}/mcp"),
            "scope": "openid mcp:query",
            "iat": now,
            "exp": now + 600,
            "jti": "test-token",
        }))
    }

    fn sign(material: &SigningMaterial, claims: &Value) -> anyhow::Result<String> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test-key".to_owned());

        Ok(encode(&header, claims, &material.key)?)
    }

    async fn spawn_app(app: axum::Router) -> anyhow::Result<(String, JoinHandle<()>)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Ok((format!("http://{address}"), server))
    }

    async fn spawn_clickhouse(
        status: StatusCode,
        body: Vec<u8>,
    ) -> anyhow::Result<(SqlExplorer, JoinHandle<()>)> {
        let app = axum::Router::new().fallback(any(move || {
            let body = body.clone();
            async move {
                Response::builder()
                    .status(status)
                    .body(Body::from(body))
                    .unwrap_or_else(|_| Response::new(Body::empty()))
            }
        }));
        let (url, server) = spawn_app(app).await?;
        let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "gold"));

        Ok((SqlExplorer::new(client, 1), server))
    }

    fn mcp_config() -> McpConfig {
        McpConfig {
            enabled: true,
            bind_addr: "127.0.0.1:0".to_owned(),
            public_url: "http://localhost:3000".to_owned(),
            allow_insecure_private_network: false,
            max_concurrent_queries: 2,
            clickhouse_user: "test_mcp".to_owned(),
            clickhouse_password: SecretString::from("test-password".to_owned()),
        }
    }

    fn assert_tool_error_contains(result: &rmcp::model::CallToolResult, expected: &str) {
        assert_eq!(result.is_error, Some(true));
        let encoded = serde_json::to_string(result).unwrap_or_default();
        assert!(
            encoded.contains(expected),
            "expected {expected:?} in {encoded}"
        );
    }

    #[tokio::test]
    async fn query_sql_returns_structured_clickhouse_json() -> anyhow::Result<()> {
        let body = serde_json::to_vec(&json!({
            "meta": [{"name": "answer", "type": "UInt8"}],
            "data": [{"answer": 1}],
            "rows": 1
        }))?;
        let (explorer, server) = spawn_clickhouse(StatusCode::OK, body).await?;

        let result = explorer
            .run_query_sql("SELECT 1 AS answer".to_owned())
            .await;

        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["row_count"].as_u64()),
            Some(1)
        );
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .map(|value| &value["rows"][0]["answer"]),
            Some(&json!(1))
        );
        let info = rmcp::ServerHandler::get_info(&explorer);
        assert_eq!(info.server_info.name, "insight-sql-explorer");
        let direct = explorer
            .query_sql(rmcp::handler::server::wrapper::Parameters(
                QuerySqlRequest {
                    sql: "SELECT 1 AS answer".to_owned(),
                },
            ))
            .await;
        assert_eq!(direct.is_error, Some(false));
        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn query_sql_rejects_invalid_and_oversized_input() {
        let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://127.0.0.1:1",
            "gold",
        ));
        let explorer = SqlExplorer::new(client, 1);

        let invalid = explorer
            .run_query_sql("DROP TABLE gold.events".to_owned())
            .await;
        assert_tool_error_contains(&invalid, "SELECT or WITH");
        let oversized = explorer.run_query_sql("x".repeat(MAX_SQL_BYTES + 1)).await;
        assert_tool_error_contains(&oversized, "request limit");
    }

    #[tokio::test]
    async fn query_execution_reports_busy_backend_and_response_failures() -> anyhow::Result<()> {
        let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://127.0.0.1:1",
            "gold",
        ));
        let busy = SqlExplorer::new(client, 0).execute("SELECT 1").await;
        assert!(matches!(busy, Err(QueryFailure::Busy)));

        let (invalid, invalid_server) =
            spawn_clickhouse(StatusCode::OK, b"not-json".to_vec()).await?;
        assert!(matches!(
            invalid.execute("SELECT 1").await,
            Err(QueryFailure::InvalidResponse(_))
        ));
        invalid_server.abort();

        let (failed, failed_server) =
            spawn_clickhouse(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()).await?;
        assert!(matches!(
            failed.execute("SELECT 1").await,
            Err(QueryFailure::ClickHouse(_))
        ));
        failed_server.abort();

        let (large, large_server) =
            spawn_clickhouse(StatusCode::OK, vec![b'x'; MAX_RESULT_BYTES + 1]).await?;
        assert!(matches!(
            large.execute("SELECT 1").await,
            Err(QueryFailure::ResultTooLarge)
        ));
        large_server.abort();
        Ok(())
    }

    #[test]
    fn configuration_requires_a_complete_safe_origin_and_credentials() {
        let mut config = mcp_config();
        assert!(validate_config(&config).is_ok());
        assert!(validate_config(&McpConfig::default()).is_ok());

        config.bind_addr = "not-an-address".to_owned();
        assert!(validate_config(&config).is_err());
        config = mcp_config();
        config.max_concurrent_queries = 0;
        assert!(validate_config(&config).is_err());
        config = mcp_config();
        config.clickhouse_user = "  ".to_owned();
        assert!(validate_config(&config).is_err());
        config = mcp_config();
        config.clickhouse_password = SecretString::from(String::new());
        assert!(validate_config(&config).is_err());

        assert!(validate_public_url("https://insight.example", false).is_ok());
        assert!(validate_public_url("http://127.0.0.1:3000", false).is_ok());
        assert!(validate_public_url("http://10.0.0.2:3000", true).is_ok());
        for invalid in [
            "http://insight.example",
            "https://user@insight.example",
            "https://insight.example/path",
            "https://insight.example?query=true",
            "https://insight.example#fragment",
        ] {
            assert!(
                validate_public_url(invalid, false).is_err(),
                "should reject {invalid}"
            );
        }
    }

    #[tokio::test]
    async fn start_accepts_valid_configuration_and_cancellation() -> anyhow::Result<()> {
        let cancellation = CancellationToken::new();

        start(
            &mcp_config(),
            "http://127.0.0.1:1",
            "gold",
            cancellation.clone(),
        )
        .await?;
        cancellation.cancel();
        tokio::task::yield_now().await;
        Ok(())
    }

    #[tokio::test]
    async fn verifier_accepts_admin_tokens_and_rejects_other_claims() -> anyhow::Result<()> {
        let material = signing_material()?;
        let app = axum::Router::new().route(
            "/.well-known/jwks.json",
            get({
                let jwks = material.jwks.clone();
                move || {
                    let jwks = jwks.clone();
                    async move { Json(jwks) }
                }
            }),
        );
        let (issuer, server) = spawn_app(app).await?;
        let verifier = TokenVerifier::new(&issuer, false)?;

        let valid = sign(&material, &claims(&issuer)?)?;
        assert!(verifier.verify(&valid).await.is_ok());

        let mut insufficient = claims(&issuer)?;
        insufficient["roles"] = json!("user");
        let insufficient = sign(&material, &insufficient)?;
        assert!(matches!(
            verifier.verify(&insufficient).await,
            Err(super::AuthFailure::InsufficientScope)
        ));

        let mut future_issued = claims(&issuer)?;
        future_issued["iat"] = json!(future_issued["exp"].as_u64().unwrap_or_default() + 1);
        let future_issued = sign(&material, &future_issued)?;
        assert!(matches!(
            verifier.verify(&future_issued).await,
            Err(super::AuthFailure::InsufficientScope)
        ));

        assert!(matches!(
            verifier.verify("not-a-token").await,
            Err(super::AuthFailure::Unauthorized)
        ));
        let token_without_kid = encode(
            &Header::new(Algorithm::ES256),
            &claims(&issuer)?,
            &material.key,
        )?;
        assert!(matches!(
            verifier.verify(&token_without_kid).await,
            Err(super::AuthFailure::Unauthorized)
        ));
        let wrong_algorithm = encode(
            &Header::new(Algorithm::HS256),
            &claims(&issuer)?,
            &EncodingKey::from_secret(b"test-secret"),
        )?;
        assert!(matches!(
            verifier.verify(&wrong_algorithm).await,
            Err(super::AuthFailure::Unauthorized)
        ));
        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn auth_middleware_returns_oauth_challenges() -> anyhow::Result<()> {
        let material = signing_material()?;
        let app = axum::Router::new().route(
            "/.well-known/jwks.json",
            get({
                let jwks = material.jwks.clone();
                move || {
                    let jwks = jwks.clone();
                    async move { Json(jwks) }
                }
            }),
        );
        let (issuer, server) = spawn_app(app).await?;
        let verifier = TokenVerifier::new(&issuer, false)?;
        let protected = axum::Router::new()
            .route("/", get(|| async { StatusCode::NO_CONTENT }))
            .layer(axum::middleware::from_fn_with_state(
                verifier.clone(),
                super::authenticate,
            ));

        let missing = protected
            .clone()
            .oneshot(Request::get("/").body(Body::empty())?)
            .await?;
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        assert!(
            missing
                .headers()
                .contains_key(axum::http::header::WWW_AUTHENTICATE)
        );

        let valid = sign(&material, &claims(&issuer)?)?;
        let allowed = protected
            .clone()
            .oneshot(
                Request::get("/")
                    .header("authorization", format!("Bearer {valid}"))
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(allowed.status(), StatusCode::NO_CONTENT);

        let mut claims = claims(&issuer)?;
        claims["scope"] = json!("openid");
        let denied = sign(&material, &claims)?;
        let denied = protected
            .oneshot(
                Request::get("/")
                    .header("authorization", format!("Bearer {denied}"))
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        server.abort();
        Ok(())
    }

    #[test]
    fn bearer_tokens_are_strictly_parsed() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer token"
                .parse()
                .unwrap_or(HeaderValue::from_static("")),
        );
        assert_eq!(bearer_token(&headers), Some("token"));

        for value in ["bearer token", "Bearer ", "Bearer two tokens"] {
            headers.insert(
                axum::http::header::AUTHORIZATION,
                value.parse().unwrap_or(HeaderValue::from_static("")),
            );
            assert_eq!(bearer_token(&headers), None);
        }
    }

    #[tokio::test]
    async fn concurrent_jwks_misses_share_one_refresh() -> anyhow::Result<()> {
        let requests = Arc::new(AtomicUsize::new(0));
        let app = axum::Router::new().route(
            "/.well-known/jwks.json",
            get({
                let requests = requests.clone();
                move || {
                    let requests = requests.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        Json(serde_json::json!({ "keys": [] }))
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let verifier = TokenVerifier::new(&format!("http://{address}"), false)?;

        let (first, second) = tokio::join!(verifier.cached_jwks(), verifier.cached_jwks());

        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert!(verifier.refresh_jwks(Some("missing-key")).await.is_ok());
        assert!(verifier.refresh_jwks(Some("missing-key")).await.is_ok());
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
        Ok(())
    }

    #[test]
    fn backend_failures_have_a_generic_public_message() {
        let clickhouse_failure = QueryFailure::ClickHouse(clickhouse::error::Error::BadResponse(
            "private backend diagnostic".to_owned(),
        ));
        let parse_error = serde_json::Error::io(std::io::Error::other("private JSON diagnostic"));
        let json_failure = QueryFailure::InvalidResponse(parse_error);

        assert_eq!(
            clickhouse_failure.public_message(),
            "query execution failed"
        );
        assert_eq!(json_failure.public_message(), "query execution failed");
        assert!(
            !clickhouse_failure
                .public_message()
                .contains("private backend diagnostic")
        );
        assert!(!json_failure.public_message().contains("JSON"));
    }

    #[test]
    fn bounded_query_failures_have_stable_public_messages() {
        assert_eq!(
            QueryFailure::Busy.public_message(),
            "the SQL explorer is busy; retry shortly"
        );
        assert_eq!(
            QueryFailure::ResultTooLarge.public_message(),
            "query result exceeded the response limit"
        );
        assert_eq!(QueryFailure::Timeout.public_message(), "query timed out");
    }
}
