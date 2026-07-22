//! OIDC relying-party client, built on the `openidconnect` crate.
//!
//! `openidconnect` owns the standards-heavy, security-critical work: discovery,
//! authorization-code + PKCE, code exchange, and id_token validation
//! (signature via JWKS, `iss`, `aud`, `nonce`, `exp`, algorithm allowlist). We
//! keep only two thin bits it doesn't surface through the `Core*` typed API:
//! the configurable tenant claim (`idp.tenant_claim`; interim — moves to the
//! Identity membership API, constructorfabric/insight#1687) and the OIDC `sid`
//! (back-channel logout index), both read from the **already-validated**
//! id_token payload; plus the RP-initiated `end_session_endpoint`, which is
//! not part of core discovery.

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};

use crate::config::IdpConfig;
use crate::identity::IdpIdentity;

/// What `authorize` hands back for the handler to stash in the login state.
pub struct AuthorizeStart {
    /// The IdP `/authorize` URL to 302 the browser to.
    pub url: String,
    /// CSRF `state` (the login-state key).
    pub state: String,
    /// Nonce to bind the eventual id_token.
    pub nonce: String,
    /// PKCE verifier to replay at code exchange.
    pub pkce_verifier: String,
}

/// The outcome of a successful callback exchange + validation.
pub struct AuthenticatedIdp {
    /// The internal-facing identity distilled from the id_token.
    pub identity: IdpIdentity,
    /// The IdP issuer (validated) — keys the back-channel logout index.
    pub issuer: String,
    /// OIDC `sid` for the back-channel logout index (when present).
    pub idp_sid: Option<String>,
    /// Raw id_token for `id_token_hint` on RP-initiated logout.
    pub id_token: String,
    /// Rotating IdP refresh token (when granted).
    pub refresh_token: Option<String>,
    /// IdP access-token lifetime in seconds (drives the refresh schedule).
    pub expires_in: Option<u64>,
}

/// One background-refresh attempt's outcome (G5 transient-vs-definitive).
#[derive(Debug)]
pub enum RefreshOutcome {
    /// The grant succeeded; store the rotated token + new expiry back.
    Refreshed {
        /// The rotated refresh token; `None` = the IdP kept the old one valid.
        new_refresh_token: Option<String>,
        /// New access-token lifetime (drives the next schedule entry).
        expires_in: Option<u64>,
    },
    /// Definitive refusal (revoked / expired / user disabled): kill the session.
    InvalidGrant(String),
    /// Transport / 5xx / 429: back off and retry, never revoke.
    Transient(String),
}

/// The OIDC client — holds config; builds the `openidconnect` client per op
/// (discovery is a cold-path login/callback concern).
#[derive(Clone)]
pub struct OidcClient {
    issuer_url: String,
    client_id: String,
    client_secret: String,
    tenant_claim: String,
    default_tenant_id: String,
    http: reqwest::Client,
}

impl OidcClient {
    /// Build the client from the `idp.*` config. Returns an error if the HTTP
    /// client can't be built.
    ///
    /// # Errors
    /// Fails when the underlying `reqwest` client cannot be constructed.
    pub fn new(idp: &IdpConfig) -> anyhow::Result<Self> {
        // Do not follow redirects: the RP must never chase the IdP's 3xx itself
        // (SSRF-safety guidance from the openidconnect docs). A total timeout is
        // mandatory (reqwest has none by default): the background refresher runs
        // each grant under a 30 s per-session lock, so a hung IdP connection
        // (half-open TCP, no RST) must fail well before that — otherwise the
        // request outlives its lock, a second worker re-runs the grant with the
        // same one-time-use refresh token, and the IdP burns it → false logout.
        // It also caps semaphore-permit hold time so hung calls can't wedge the
        // whole refresher (G5).
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .context("build OIDC HTTP client")?;
        Ok(Self {
            issuer_url: idp.issuer_url.trim_end_matches('/').to_owned(),
            client_id: idp.client_id.clone(),
            client_secret: idp.client_secret.clone(),
            tenant_claim: idp.tenant_claim.clone(),
            default_tenant_id: idp.default_tenant_id.clone(),
            http,
        })
    }

    /// Fetch the provider discovery metadata.
    async fn metadata(&self) -> anyhow::Result<CoreProviderMetadata> {
        let issuer = IssuerUrl::new(self.issuer_url.clone()).context("invalid issuer_url")?;
        CoreProviderMetadata::discover_async(issuer, &self.http)
            .await
            .context("OIDC discovery")
    }

    /// Confidential-client secret, if configured (public clients omit it).
    fn secret(&self) -> Option<ClientSecret> {
        (!self.client_secret.is_empty()).then(|| ClientSecret::new(self.client_secret.clone()))
    }

    /// Begin login: build the `/authorize` URL with PKCE (S256), a random state
    /// and nonce.
    ///
    /// # Errors
    /// Fails on discovery / URL-construction errors.
    pub async fn authorize(
        &self,
        redirect_uri: &str,
        scopes: &[String],
    ) -> anyhow::Result<AuthorizeStart> {
        // Built inline (not via a helper) so the endpoint type-state markers
        // from `from_provider_metadata` + `set_redirect_uri` are preserved.
        let client = CoreClient::from_provider_metadata(
            self.metadata().await?,
            ClientId::new(self.client_id.clone()),
            self.secret(),
        )
        .set_redirect_uri(
            RedirectUrl::new(redirect_uri.to_owned()).context("invalid redirect_uri")?,
        );
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();

        let mut builder = client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            openidconnect::CsrfToken::new_random,
            Nonce::new_random,
        );
        // `openid` is added by the flow; add the rest from config.
        for scope in scopes.iter().filter(|s| s.as_str() != "openid") {
            builder = builder.add_scope(Scope::new(scope.clone()));
        }
        let (url, state, nonce) = builder.set_pkce_challenge(challenge).url();

        Ok(AuthorizeStart {
            url: url.to_string(),
            state: state.secret().clone(),
            nonce: nonce.secret().clone(),
            pkce_verifier: verifier.secret().clone(),
        })
    }

    /// Exchange the code (with the PKCE verifier), validate the id_token, and
    /// distill the principal.
    ///
    /// # Errors
    /// Fails on transport errors, a token-endpoint error, or id_token
    /// validation failure (signature / iss / aud / nonce / exp).
    pub async fn exchange_code_pkce(
        &self,
        redirect_uri: &str,
        code: &str,
        pkce_verifier: &str,
        expected_nonce: &str,
    ) -> anyhow::Result<AuthenticatedIdp> {
        let client = CoreClient::from_provider_metadata(
            self.metadata().await?,
            ClientId::new(self.client_id.clone()),
            self.secret(),
        )
        .set_redirect_uri(
            RedirectUrl::new(redirect_uri.to_owned()).context("invalid redirect_uri")?,
        );
        let token = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .context("build code-exchange request")?
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
            .request_async(&self.http)
            .await
            // Surface the IdP's OAuth error + description (ServerResponse), not a
            // bare "code exchange", so the failure is diagnosable.
            .map_err(|e| {
                use openidconnect::RequestTokenError::ServerResponse;
                match &e {
                    ServerResponse(r) => anyhow::anyhow!(
                        "idp token endpoint rejected the code: {}{}",
                        r.error(),
                        r.error_description()
                            .map(|d| format!(" — {d}"))
                            .unwrap_or_default(),
                    ),
                    _ => anyhow::anyhow!("code exchange transport/parse error: {e}"),
                }
            })?;

        let id_token = token.id_token().context("token response has no id_token")?;
        let claims = id_token
            .claims(
                &client.id_token_verifier(),
                &Nonce::new(expected_nonce.to_owned()),
            )
            .context("id_token validation failed")?;

        // Standard claims from the typed API.
        let sub = claims.subject().to_string();
        let issuer = claims.issuer().to_string();
        let email = claims.email().map(|e| e.to_string()).unwrap_or_default();

        // Non-standard claims read from the already-validated payload. One and
        // only one tenant per token (EPIC #1583): the claim name is per-IdP
        // (`tenant_id` on fakeidp/Keycloak, `tid` on Entra); claim-less IdPs
        // (Okta) fall back to the configured default tenant; empty = downstream
        // fails closed.
        let raw = id_token.to_string();
        let mut tenant_id = payload_tenant(&raw, &self.tenant_claim);
        if tenant_id.is_empty() && !self.default_tenant_id.is_empty() {
            tracing::debug!(tenant_id = %self.default_tenant_id, "id_token carries no tenant claim; using idp.default_tenant_id");
            tenant_id.clone_from(&self.default_tenant_id);
        }
        let idp_sid = payload_string(&raw, "sid");

        Ok(AuthenticatedIdp {
            identity: IdpIdentity {
                sub,
                email,
                tenant_id,
            },
            issuer,
            idp_sid,
            id_token: raw,
            refresh_token: token.refresh_token().map(|r| r.secret().clone()),
            expires_in: token.expires_in().map(|d| d.as_secs()),
        })
    }

    /// Run a `refresh_token` grant for the background refresher (G5). The
    /// outcome distinguishes a **definitive** IdP verdict (`invalid_grant`:
    /// revoked / expired / user disabled → the caller kills the session) from
    /// **transient** failures (network, 5xx, 429 → the caller backs off and
    /// retries; nobody is logged out by a blip).
    pub async fn refresh_grant(&self, refresh_token: &str) -> RefreshOutcome {
        use openidconnect::RequestTokenError::ServerResponse;
        use openidconnect::core::CoreErrorResponseType;

        let metadata = match self.metadata().await {
            Ok(m) => m,
            Err(e) => return RefreshOutcome::Transient(format!("discovery: {e:#}")),
        };
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(self.client_id.clone()),
            self.secret(),
        );
        let rt = openidconnect::RefreshToken::new(refresh_token.to_owned());
        let request = match client.exchange_refresh_token(&rt) {
            Ok(r) => r,
            Err(e) => return RefreshOutcome::Transient(format!("build refresh request: {e}")),
        };
        let result = request.request_async(&self.http).await;

        match result {
            Ok(token) => RefreshOutcome::Refreshed {
                // Most IdPs rotate (one-time-use); keeping the old token when
                // none is returned matches RFC 6749 §6.
                new_refresh_token: token.refresh_token().map(|r| r.secret().clone()),
                expires_in: token.expires_in().map(|d| d.as_secs()),
            },
            Err(ServerResponse(r)) if *r.error() == CoreErrorResponseType::InvalidGrant => {
                RefreshOutcome::InvalidGrant(
                    r.error_description()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                )
            }
            // Every other token-endpoint error (invalid_client, 5xx-shaped
            // bodies, 429) and all transport/parse errors are transient: fail
            // open on transport, fail closed only on the definitive verdict.
            Err(e) => RefreshOutcome::Transient(format!("{e}")),
        }
    }

    /// The IdP issuer URL this client trusts (back-channel `iss` check).
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer_url
    }

    /// The registered client id (back-channel `aud` check).
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Fetch the IdP's JWKS (via discovery) for back-channel `logout_token`
    /// verification. Cold path — back-channel logout is rare — so no cache:
    /// a fresh fetch also picks up IdP key rotation immediately.
    ///
    /// # Errors
    /// Fails when discovery or the JWKS endpoint is unreachable / malformed.
    pub async fn idp_jwks(&self) -> anyhow::Result<jsonwebtoken::jwk::JwkSet> {
        #[derive(serde::Deserialize)]
        struct Disco {
            jwks_uri: String,
        }
        let disco: Disco = self
            .http
            .get(format!(
                "{}/.well-known/openid-configuration",
                self.issuer_url
            ))
            .send()
            .await
            .context("fetch IdP discovery")?
            .json()
            .await
            .context("decode IdP discovery")?;
        self.http
            .get(&disco.jwks_uri)
            .send()
            .await
            .context("fetch IdP JWKS")?
            .json()
            .await
            .context("decode IdP JWKS")
    }

    /// Build the RP-initiated logout URL. `end_session_endpoint` is not part of
    /// core discovery, so it is fetched here directly. Returns `None` when the
    /// IdP advertises no endpoint.
    #[must_use]
    pub async fn rp_logout_url(
        &self,
        id_token_hint: &str,
        post_logout_redirect_uri: &str,
    ) -> Option<String> {
        #[derive(serde::Deserialize)]
        struct Disco {
            end_session_endpoint: Option<String>,
        }
        let disco: Disco = self
            .http
            .get(format!(
                "{}/.well-known/openid-configuration",
                self.issuer_url
            ))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        let endpoint = disco.end_session_endpoint?;
        let mut url = url::Url::parse(&endpoint).ok()?;
        url.query_pairs_mut()
            .append_pair("id_token_hint", id_token_hint)
            .append_pair("post_logout_redirect_uri", post_logout_redirect_uri);
        Some(url.into())
    }
}

/// Read the single tenant from an (already-validated) compact JWT payload.
/// Accepts a plain string (`tenant_id` on fakeidp/Keycloak, `tid` on Entra); a
/// string array is tolerated by taking its first entry (a Keycloak multivalued
/// mapper). Anything else yields empty (→ fail closed downstream).
fn payload_tenant(jwt: &str, field: &str) -> String {
    match payload(jwt).as_ref().and_then(|v| v.get(field).cloned()) {
        Some(serde_json::Value::String(s)) => s,
        Some(v) => serde_json::from_value::<Vec<String>>(v)
            .ok()
            .and_then(|mut t| (!t.is_empty()).then(|| t.remove(0)))
            .unwrap_or_default(),
        None => String::new(),
    }
}

/// Read a string claim from an (already-validated) compact JWT payload.
fn payload_string(jwt: &str, field: &str) -> Option<String> {
    payload(jwt)?
        .get(field)?
        .as_str()
        .map(std::borrow::ToOwned::to_owned)
}

/// Decode the payload segment of a compact JWT to JSON (no verification — the
/// caller has already validated the token via `openidconnect`).
fn payload(jwt: &str) -> Option<serde_json::Value> {
    let segment = jwt.split('.').nth(1)?;
    let bytes = B64.decode(segment).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Compact-JWT shell around a claims object (header/signature are dummies —
    /// `payload` only reads the middle segment).
    fn jwt_with(claims: &serde_json::Value) -> String {
        let body = B64.encode(serde_json::to_vec(claims).unwrap());
        format!("e30.{body}.sig")
    }

    #[test]
    fn tenant_claim_string_and_array_shapes() {
        // Canonical shape: a plain string (`tenant_id` ours, `tid` Entra).
        let jwt = jwt_with(&serde_json::json!({"tenant_id": "t1"}));
        assert_eq!(payload_tenant(&jwt, "tenant_id"), "t1");
        let jwt = jwt_with(&serde_json::json!({"tid": "dir-guid"}));
        assert_eq!(payload_tenant(&jwt, "tid"), "dir-guid");

        // Tolerated: an array (Keycloak multivalued mapper) — first entry wins.
        let jwt = jwt_with(&serde_json::json!({"tenant_id": ["t1", "t2"]}));
        assert_eq!(payload_tenant(&jwt, "tenant_id"), "t1");
    }

    #[test]
    fn tenant_claim_absent_or_malformed_is_empty() {
        let jwt = jwt_with(&serde_json::json!({"sub": "u1"}));
        assert!(payload_tenant(&jwt, "tenant_id").is_empty());

        // Wrong shape (number / mixed array) never panics, yields empty.
        let jwt = jwt_with(&serde_json::json!({"tenant_id": 42}));
        assert!(payload_tenant(&jwt, "tenant_id").is_empty());
        let jwt = jwt_with(&serde_json::json!({"tenant_id": ["t1", 2]}));
        assert!(payload_tenant(&jwt, "tenant_id").is_empty());
    }
}
