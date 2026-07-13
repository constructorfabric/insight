//! `ServiceTokenClient` — the client side of the service-token flow (§10 G1 /
//! DD-AUTH-05). This is what analytics and background jobs use (step 07) to
//! obtain a gateway JWT without a user request.
//!
//! It holds the service's private key, mints a short-lived RFC 7523
//! `private_key_jwt` assertion (`iss = sub = <service>`, `aud = <token endpoint
//! URL>`, `jti`, `exp`), exchanges it at the authenticator's `POST
//! /internal/token` for a normal gateway JWT, caches that token, and re-requests
//! it ahead of expiry (at 4/5 of the token's TTL — the same reissue-ahead
//! pattern the authenticator uses everywhere). The private key never leaves the
//! process; only the short-lived assertion travels, and only public keys live
//! in the authenticator's registry.

use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

/// Default lifetime of a minted client assertion (seconds). Well within the
/// authenticator's 60 s cap, with room for a little clock skew.
const DEFAULT_ASSERTION_TTL_SECONDS: u64 = 30;

/// A token fetched from the authenticator: the raw JWT and its lifetime.
#[derive(Debug, Clone)]
pub struct FetchedToken {
    /// The gateway JWT (no `Bearer ` prefix).
    pub access_token: String,
    /// Seconds until the token expires, as reported by the endpoint.
    pub expires_in: u64,
}

/// The token-endpoint response shape (OAuth2 `client_credentials`).
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

/// The RFC 7523 assertion claims this client signs.
#[derive(Serialize)]
struct AssertionClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    jti: String,
    iat: u64,
    exp: u64,
}

/// A cached bearer value and the time at which it should be refreshed.
struct Cached {
    /// The full `Bearer <jwt>` header value.
    bearer: String,
    /// Epoch second at/after which `bearer()` re-fetches (4/5 of the TTL).
    refresh_at: u64,
}

/// Mints assertions, fetches + caches a service token, and hands out a bearer.
///
/// Construct once per (service, key) and share it (`Arc`) — it is internally
/// synchronized and the cache is shared across callers, so concurrent
/// `bearer()` calls collapse onto a single in-flight fetch.
pub struct ServiceTokenClient {
    service: String,
    /// Token endpoint URL; also the `aud` the assertion is minted for.
    endpoint: String,
    encoding: EncodingKey,
    assertion_ttl_seconds: u64,
    http: reqwest::Client,
    cache: Mutex<Option<Cached>>,
}

impl ServiceTokenClient {
    /// Build a client from a PKCS#8 EC P-256 private-key PEM.
    ///
    /// `service` is the registry name (becomes the assertion `iss`/`sub`);
    /// `endpoint` is the authenticator token endpoint URL (e.g.
    /// `http://authenticator:8093/internal/token`), used verbatim as the POST
    /// target and the assertion `aud`.
    ///
    /// # Errors
    /// Fails when the PEM is not a usable EC private key, or the HTTP client
    /// cannot be built.
    pub fn from_key_pem(
        service: impl Into<String>,
        private_key_pem: &str,
        endpoint: impl Into<String>,
    ) -> Result<Self, CanonicalError> {
        let encoding = EncodingKey::from_ec_pem(private_key_pem.as_bytes()).map_err(|e| {
            CanonicalError::internal(format!("invalid service private key PEM: {e}")).create()
        })?;
        let http = reqwest::Client::builder().build().map_err(|e| {
            CanonicalError::internal(format!("build service-token HTTP client: {e}")).create()
        })?;
        Ok(Self {
            service: service.into(),
            endpoint: endpoint.into(),
            encoding,
            assertion_ttl_seconds: DEFAULT_ASSERTION_TTL_SECONDS,
            http,
            cache: Mutex::new(None),
        })
    }

    /// Build a client by reading the private key from `path`.
    ///
    /// # Errors
    /// Fails when the file cannot be read or is not a usable EC private key.
    pub fn from_key_file(
        service: impl Into<String>,
        private_key_path: impl AsRef<std::path::Path>,
        endpoint: impl Into<String>,
    ) -> Result<Self, CanonicalError> {
        let path = private_key_path.as_ref();
        let pem = std::fs::read_to_string(path).map_err(|e| {
            CanonicalError::internal(format!("read service private key {}: {e}", path.display()))
                .create()
        })?;
        Self::from_key_pem(service, &pem, endpoint)
    }

    /// Override the minted-assertion lifetime (seconds). Must stay at or below
    /// the authenticator's cap (60 s by default).
    #[must_use]
    pub fn with_assertion_ttl_seconds(mut self, ttl: u64) -> Self {
        self.assertion_ttl_seconds = ttl;
        self
    }

    /// Mint and sign a fresh RFC 7523 client assertion. Public so tests (and
    /// advanced callers) can drive the raw endpoint — e.g. to exercise replay
    /// by posting the same assertion twice.
    ///
    /// # Errors
    /// Fails only on an internal signing error.
    pub fn make_assertion(&self) -> Result<String, CanonicalError> {
        let now = now_secs();
        let claims = AssertionClaims {
            iss: &self.service,
            sub: &self.service,
            aud: &self.endpoint,
            jti: Uuid::now_v7().to_string(),
            iat: now,
            exp: now + self.assertion_ttl_seconds,
        };
        encode(&Header::new(Algorithm::ES256), &claims, &self.encoding)
            .map_err(|e| CanonicalError::internal(format!("sign client assertion: {e}")).create())
    }

    /// Fetch a fresh token (uncached). Pass tenant ids to request a
    /// tenant-scoped token (allowed only if the registry entry permits it;
    /// otherwise the endpoint refuses and this returns an error).
    ///
    /// # Errors
    /// Returns `ServiceUnavailable` on a transport failure and `Internal` when
    /// the endpoint answers non-2xx or an undecodable body.
    pub async fn fetch(&self, tenants: &[String]) -> Result<FetchedToken, CanonicalError> {
        let assertion = self.make_assertion()?;
        self.post(&assertion, tenants).await
    }

    /// POST a (possibly externally-minted) assertion to the token endpoint.
    ///
    /// # Errors
    /// As [`fetch`](Self::fetch).
    pub async fn post(
        &self,
        assertion: &str,
        tenants: &[String],
    ) -> Result<FetchedToken, CanonicalError> {
        let mut form = vec![
            ("grant_type", "client_credentials".to_owned()),
            (
                "client_assertion_type",
                "urn:ietf:params:oauth:client-assertion-type:jwt-bearer".to_owned(),
            ),
            ("client_assertion", assertion.to_owned()),
        ];
        if !tenants.is_empty() {
            form.push(("tenants", tenants.join(",")));
        }

        let resp = self
            .http
            .post(&self.endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| {
                CanonicalError::service_unavailable()
                    .with_detail(format!("service-token request failed: {e}"))
                    .create()
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CanonicalError::internal(format!(
                "service-token endpoint returned {status}: {body}"
            ))
            .create());
        }
        let body: TokenResponse = resp.json().await.map_err(|e| {
            CanonicalError::internal(format!("decode service-token response: {e}")).create()
        })?;
        Ok(FetchedToken {
            access_token: body.access_token,
            expires_in: body.expires_in,
        })
    }

    /// The cross-tenant service bearer, ready for an `Authorization` header
    /// (`"Bearer <jwt>"`). Served from cache until 4/5 of its TTL has elapsed,
    /// then re-fetched ahead of expiry. Concurrent callers share one fetch.
    ///
    /// # Errors
    /// As [`fetch`](Self::fetch), when a refresh is needed and fails.
    pub async fn bearer(&self) -> Result<String, CanonicalError> {
        let mut cache = self.cache.lock().await;
        let now = now_secs();
        if let Some(c) = cache.as_ref()
            && now < c.refresh_at
        {
            return Ok(c.bearer.clone());
        }

        let token = self.fetch(&[]).await?;
        let bearer = format!("Bearer {}", token.access_token);
        // Reissue ahead of expiry: refresh once 4/5 of the lifetime has passed,
        // leaving the last fifth as travel margin. A zero/short TTL degrades to
        // fetch-every-call, which is safe (just chattier).
        let refresh_at = now + token.expires_in.saturating_mul(4) / 5;
        *cache = Some(Cached {
            bearer: bearer.clone(),
            refresh_at,
        });
        Ok(bearer)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
