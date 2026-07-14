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

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

/// Default lifetime of a minted client assertion (seconds). Well within the
/// authenticator's 60 s cap, with room for a little clock skew.
const DEFAULT_ASSERTION_TTL_SECONDS: u64 = 30;

/// Connect timeout for the token endpoint — bound so a hung authenticator can
/// never wedge a caller.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Overall request timeout for the token exchange.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

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

/// A cached bearer value with its refresh and hard-expiry times.
#[derive(Clone)]
struct Cached {
    /// The full `Bearer <jwt>` header value.
    bearer: String,
    /// Epoch second at/after which `bearer()` re-fetches (4/5 of the TTL).
    refresh_at: u64,
    /// Epoch second at which the token truly expires — after this it must not
    /// be served even as a stale fallback.
    expires_at: u64,
}

/// Mints assertions, fetches + caches service tokens, and hands out bearers.
///
/// Construct once per (service, key) and share it (`Arc`). The cache is a
/// read-mostly `RwLock` keyed by tenant scope: `bearer()` takes only a short
/// read lock to check the cache, releases it, does the network fetch with **no
/// lock held**, then takes a brief write lock to store. Concurrent refreshes
/// may each fetch (no single-flight) — an acceptable trade for never blocking a
/// caller behind another's HTTP round-trip.
pub struct ServiceTokenClient {
    service: String,
    /// Token endpoint URL; also the `aud` the assertion is minted for.
    endpoint: String,
    encoding: EncodingKey,
    assertion_ttl_seconds: u64,
    http: reqwest::Client,
    /// Cache keyed by the (sorted) requested tenant scope. Service tokens are
    /// always tenant-scoped, so there is one entry per tenant set in use.
    cache: RwLock<HashMap<Vec<String>, Cached>>,
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
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| {
                CanonicalError::internal(format!("build service-token HTTP client: {e}")).create()
            })?;
        Ok(Self {
            service: service.into(),
            endpoint: endpoint.into(),
            encoding,
            assertion_ttl_seconds: DEFAULT_ASSERTION_TTL_SECONDS,
            http,
            cache: RwLock::new(HashMap::new()),
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

    /// A service bearer for `tenants`, ready for an `Authorization` header
    /// (`"Bearer <jwt>"`). Service tokens are always tenant-scoped, so a tenant
    /// must be named (the endpoint rejects an empty scope). Served from a
    /// per-scope cache until 4/5 of the TTL has elapsed, then re-fetched ahead
    /// of expiry; if the refresh fails but the cached token has not truly
    /// expired, the stale-but-valid token is served (resilience across a short
    /// authenticator outage).
    ///
    /// The lock is held only to read and to write the cache — never across the
    /// network fetch.
    ///
    /// # Errors
    /// As [`fetch`](Self::fetch), when a refresh is needed and both the fetch
    /// and the stale fallback are unavailable.
    pub async fn bearer(&self, tenants: &[String]) -> Result<String, CanonicalError> {
        let key = cache_key(tenants);

        // Fast path: short read lock, then release before any await.
        {
            let cache = self.cache.read().await;
            if let Some(c) = cache.get(&key)
                && now_secs() < c.refresh_at
            {
                return Ok(c.bearer.clone());
            }
        }

        // Refresh with no lock held.
        match self.fetch(tenants).await {
            Ok(token) => {
                let bearer = format!("Bearer {}", token.access_token);
                // Anchor the cache times to when the token was RECEIVED, not to
                // before the fetch — the round-trip can take up to the request
                // timeout, and using a pre-fetch `now` would expire the entry
                // early. Reissue once 4/5 of the lifetime has passed, leaving
                // the last fifth as travel margin.
                let received_at = now_secs();
                let entry = Cached {
                    bearer: bearer.clone(),
                    refresh_at: received_at + token.expires_in.saturating_mul(4) / 5,
                    expires_at: received_at + token.expires_in,
                };
                self.cache.write().await.insert(key, entry);
                Ok(bearer)
            }
            Err(e) => {
                // Serve stale-but-valid on a refresh failure.
                let cache = self.cache.read().await;
                if let Some(c) = cache.get(&key)
                    && now_secs() < c.expires_at
                {
                    return Ok(c.bearer.clone());
                }
                Err(e)
            }
        }
    }
}

/// Cache key for a tenant scope: the ids sorted so `[a,b]` and `[b,a]` share
/// one entry.
fn cache_key(tenants: &[String]) -> Vec<String> {
    let mut key = tenants.to_vec();
    key.sort();
    key
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
