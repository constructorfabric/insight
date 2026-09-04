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
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::config::McpConfig;
use crate::sql_explorer::executor::{MAX_REQUEST_BODY_BYTES, QueryExecutor};

const MAX_JWKS_BYTES: usize = 1024 * 1024;
const JWKS_REFRESH_COOLDOWN: Duration = Duration::from_secs(5);
const MCP_SCOPE: &str = "mcp:query";

#[derive(Debug, Deserialize, JsonSchema)]
struct QuerySqlRequest {
    /// One ClickHouse SELECT or WITH statement. Multiple CTEs and nested queries are allowed.
    sql: String,
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

#[derive(Clone)]
struct SqlExplorer {
    executor: QueryExecutor,
    tool_router: ToolRouter<Self>,
}

impl SqlExplorer {
    fn new(executor: QueryExecutor) -> Self {
        Self {
            executor,
            tool_router: Self::tool_router(),
        }
    }

    async fn run_query_sql(&self, sql: String) -> CallToolResult {
        match self
            .executor
            .query(sql, |value| Ok(CallToolResult::structured(value)))
            .await
        {
            Ok(result) => result,
            Err(error) => tool_error(error.public_message()),
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

pub(crate) fn router(
    config: &McpConfig,
    executor: QueryExecutor,
    cancellation_token: CancellationToken,
) -> anyhow::Result<Router> {
    let explorer = SqlExplorer::new(executor);
    let service: StreamableHttpService<SqlExplorer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(explorer.clone()),
            Arc::default(),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(false)
                .with_json_response(true)
                .with_allowed_hosts(["localhost"])
                .with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES)
                .with_cancellation_token(cancellation_token),
        );
    let auth = TokenVerifier::new(&config.public_url, config.allow_insecure_private_network)?;
    Ok(Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(auth, authenticate)))
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

pub(crate) fn validate_public_url(
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
mod tests;
