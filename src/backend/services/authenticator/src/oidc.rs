//! OIDC relying-party client, built on the `openidconnect` crate.
//!
//! `openidconnect` owns the standards-heavy, security-critical work: discovery,
//! authorization-code + PKCE, code exchange, and id_token validation
//! (signature via JWKS, `iss`, `aud`, `nonce`, `exp`, algorithm allowlist). We
//! keep only two thin bits it doesn't surface through the `Core*` typed API:
//! the non-standard `tenants` claim (interim — moves to the Identity membership
//! API, constructorfabric/insight#1687) and the OIDC `sid` (back-channel logout
//! index), both read from the **already-validated** id_token payload; plus the
//! RP-initiated `end_session_endpoint`, which is not part of core discovery.

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};

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

/// The OIDC client — holds config; builds the `openidconnect` client per op
/// (discovery is a cold-path login/callback concern).
#[derive(Clone)]
pub struct OidcClient {
    issuer_url: String,
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl OidcClient {
    /// Build the client. Returns an error if the HTTP client can't be built.
    ///
    /// # Errors
    /// Fails when the underlying `reqwest` client cannot be constructed.
    pub fn new(issuer_url: &str, client_id: &str, client_secret: &str) -> anyhow::Result<Self> {
        // Do not follow redirects: the RP must never chase the IdP's 3xx itself
        // (SSRF-safety guidance from the openidconnect docs).
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("build OIDC HTTP client")?;
        Ok(Self {
            issuer_url: issuer_url.trim_end_matches('/').to_owned(),
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
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
            .context("code exchange")?;

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

        // Non-standard claims read from the already-validated payload.
        let raw = id_token.to_string();
        let tenants = payload_string_array(&raw, "tenants");
        let idp_sid = payload_string(&raw, "sid");

        Ok(AuthenticatedIdp {
            identity: IdpIdentity {
                sub,
                email,
                tenants,
            },
            issuer,
            idp_sid,
            id_token: raw,
            refresh_token: token.refresh_token().map(|r| r.secret().clone()),
            expires_in: token.expires_in().map(|d| d.as_secs()),
        })
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

/// Read a string-array claim from an (already-validated) compact JWT payload.
fn payload_string_array(jwt: &str, field: &str) -> Vec<String> {
    payload(jwt)
        .as_ref()
        .and_then(|v| v.get(field))
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default()
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
