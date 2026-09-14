use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};

#[cfg(test)]
mod tests;

const MAX_JWKS_BYTES: usize = 1024 * 1024;
const JWKS_REFRESH_COOLDOWN: Duration = Duration::from_secs(5);

/// The scope a grant for this server carries. The read-only warehouse server
/// has its own, and neither is valid for the other's resource.
pub(crate) const MCP_SCOPE: &str = "mcp:author";

/// The path this server answers on, which is also the resource its tokens are
/// minted for.
pub(crate) const MCP_PATH: &str = "/mcp/v3";

#[derive(Debug, Deserialize)]
pub(crate) struct McpAccessClaims {
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

#[derive(Debug)]
pub(crate) enum AuthFailure {
    Unauthorized,
    InsufficientScope,
    Unavailable,
}

#[derive(Clone)]
pub(crate) struct TokenVerifier {
    inner: Arc<TokenVerifierInner>,
}

impl std::fmt::Debug for TokenVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenVerifier")
            .field("issuer", &self.inner.issuer)
            .finish_non_exhaustive()
    }
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

impl TokenVerifier {
    pub(crate) fn new(
        public_url: &str,
        jwks_url: &str,
        allow_insecure_private_network: bool,
    ) -> anyhow::Result<Self> {
        validate_public_url(public_url, allow_insecure_private_network)?;
        let issuer = public_url.trim_end_matches('/').to_owned();
        // Only where the keys come from. The token still has to name the
        // advertised origin, which is checked below against `issuer`.
        let jwks_url = if jwks_url.trim().is_empty() {
            format!("{issuer}/.well-known/jwks.json")
        } else {
            jwks_url.trim().to_owned()
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()?;

        Ok(Self {
            inner: Arc::new(TokenVerifierInner {
                audience: format!("{issuer}{MCP_PATH}"),
                resource_metadata: format!(
                    "{issuer}/.well-known/oauth-protected-resource{MCP_PATH}"
                ),
                jwks_url,
                issuer,
                client,
                jwks: RwLock::new(None),
                jwks_refresh: Mutex::new(JwksRefresh::default()),
            }),
        })
    }

    /// The `admin` role is read off the token, not from identity: the gateway
    /// forwards an MCP bearer unchanged, and only an administrator is granted one.
    pub(crate) async fn verify(&self, token: &str) -> Result<McpAccessClaims, AuthFailure> {
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

    pub(crate) fn challenge(&self, status: StatusCode, error: &str) -> Response {
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
        // INVARIANT: holding the gate across the fetch permits one refresh at a time.
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
                tracing::warn!(%error, "could not fetch the MCP JWKS");
                AuthFailure::Unavailable
            })?
            .error_for_status()
            .map_err(|error| {
                tracing::warn!(%error, "the MCP JWKS endpoint returned an error");
                AuthFailure::Unavailable
            })?;

        if response
            .content_length()
            .is_some_and(|length| length > MAX_JWKS_BYTES as u64)
        {
            return Err(AuthFailure::Unavailable);
        }

        let bytes = response.bytes().await.map_err(|error| {
            tracing::warn!(%error, "could not read the MCP JWKS response");
            AuthFailure::Unavailable
        })?;
        if bytes.len() > MAX_JWKS_BYTES {
            return Err(AuthFailure::Unavailable);
        }

        let jwks = serde_json::from_slice::<JwkSet>(&bytes).map_err(|error| {
            tracing::warn!(%error, "could not parse the MCP JWKS response");
            AuthFailure::Unavailable
        })?;

        *self.inner.jwks.write().await = Some(jwks.clone());

        Ok(jwks)
    }
}

pub(crate) async fn authenticate(
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

pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty() && !token.chars().any(char::is_whitespace))
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
