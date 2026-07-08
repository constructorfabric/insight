//! OIDC client — authorization-code + PKCE, id_token validation, RP-logout URL.
//!
//! Encapsulates the OIDC protocol against the customer IdP (and, in dev/CI, the
//! in-repo `fakeidp`). The code+PKCE client is a new build; token verification
//! uses the IdP's JWKS. Discovery is fetched per operation for step 04 (login /
//! callback are cold-path); a cache is a later optimization.

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use rand::RngCore as _;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::identity::IdpIdentity;

/// OIDC discovery document (the fields we consume).
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    #[serde(default)]
    pub end_session_endpoint: Option<String>,
}

/// Token endpoint response.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub id_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// The validated id_token claims we care about.
#[derive(Debug, Clone, Deserialize)]
struct IdTokenClaims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    sid: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    tenants: Vec<String>,
}

/// A single JWK (RSA — what fakeidp and most OIDC IdPs use for id_tokens).
#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kid: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

/// PKCE pair: the high-entropy verifier and its S256 challenge.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// Generate a fresh PKCE pair (S256).
    #[must_use]
    pub fn generate() -> Self {
        let mut raw = [0u8; 48];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let verifier = B64.encode(raw);
        let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

/// The outcome of a successful callback exchange + validation.
pub struct AuthenticatedIdp {
    /// The internal-facing identity distilled from the id_token.
    pub identity: IdpIdentity,
    /// The IdP issuer (from discovery) — keys the back-channel logout index.
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

/// The OIDC client — reuses one `reqwest::Client`.
#[derive(Clone)]
pub struct OidcClient {
    issuer_url: String,
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl OidcClient {
    #[must_use]
    pub fn new(issuer_url: &str, client_id: &str, client_secret: &str) -> Self {
        Self {
            issuer_url: issuer_url.trim_end_matches('/').to_owned(),
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            http: reqwest::Client::new(),
        }
    }

    /// Fetch the discovery document.
    ///
    /// # Errors
    /// Fails when the IdP is unreachable or the document is malformed.
    pub async fn discover(&self) -> anyhow::Result<Discovery> {
        let url = format!("{}/.well-known/openid-configuration", self.issuer_url);
        let d: Discovery = self
            .http
            .get(&url)
            .send()
            .await
            .context("OIDC discovery request")?
            .error_for_status()
            .context("OIDC discovery status")?
            .json()
            .await
            .context("decode OIDC discovery")?;
        Ok(d)
    }

    /// Build the `/authorize` redirect URL for a code+PKCE start.
    ///
    /// # Errors
    /// Fails when the authorization endpoint is not a valid URL.
    pub fn authorize_url(
        &self,
        d: &Discovery,
        redirect_uri: &str,
        scopes: &[String],
        state: &str,
        nonce: &str,
        pkce: &Pkce,
    ) -> anyhow::Result<String> {
        let mut url = url::Url::parse(&d.authorization_endpoint)
            .context("parse authorization_endpoint")?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", &scopes.join(" "))
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url.into())
    }

    /// Exchange an authorization code with its PKCE verifier, validate the
    /// id_token against the IdP JWKS, and distill the principal.
    ///
    /// # Errors
    /// Fails on transport errors, a token-endpoint error, or id_token
    /// validation failure (signature / iss / aud / nonce / exp).
    pub async fn exchange_code_pkce(
        &self,
        d: &Discovery,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
        expected_nonce: &str,
    ) -> anyhow::Result<AuthenticatedIdp> {
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", self.client_id.as_str()),
            ("code_verifier", code_verifier),
        ];
        if !self.client_secret.is_empty() {
            form.push(("client_secret", self.client_secret.as_str()));
        }

        let tokens: TokenResponse = self
            .http
            .post(&d.token_endpoint)
            .form(&form)
            .send()
            .await
            .context("token exchange request")?
            .error_for_status()
            .context("token exchange status")?
            .json()
            .await
            .context("decode token response")?;

        let claims = self
            .validate_id_token(d, &tokens.id_token, expected_nonce)
            .await?;

        Ok(AuthenticatedIdp {
            identity: IdpIdentity {
                sub: claims.sub.clone(),
                email: claims.email.clone().unwrap_or_default(),
                tenants: claims.tenants.clone(),
            },
            issuer: d.issuer.clone(),
            idp_sid: claims.sid.clone(),
            id_token: tokens.id_token,
            refresh_token: tokens.refresh_token,
            expires_in: tokens.expires_in,
        })
    }

    /// Validate an id_token: RSA signature via JWKS, `iss`, `aud`, `exp`, and
    /// the `nonce` binding.
    async fn validate_id_token(
        &self,
        d: &Discovery,
        id_token: &str,
        expected_nonce: &str,
    ) -> anyhow::Result<IdTokenClaims> {
        let header = decode_header(id_token).context("decode id_token header")?;
        let jwks: Jwks = self
            .http
            .get(&d.jwks_uri)
            .send()
            .await
            .context("JWKS request")?
            .error_for_status()
            .context("JWKS status")?
            .json()
            .await
            .context("decode JWKS")?;

        let jwk = jwks
            .keys
            .iter()
            .find(|k| header.kid.is_none() || k.kid == header.kid)
            .context("no matching JWK for id_token kid")?;
        let (n, e) = (
            jwk.n.as_deref().context("JWK missing n")?,
            jwk.e.as_deref().context("JWK missing e")?,
        );
        let decoding = DecodingKey::from_rsa_components(n, e).context("build RSA decoding key")?;

        // Pin the algorithm to RS256 (the RSA JWK we just built) rather than
        // trusting the token's own `alg` header — RFC 8725 algorithm-confusion
        // guard. `Validation::new` sets `algorithms = [RS256]`.
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.client_id]);
        validation.set_issuer(&[&d.issuer]);
        validation.validate_exp = true;

        let data =
            decode::<IdTokenClaims>(id_token, &decoding, &validation).context("id_token invalid")?;

        // Nonce binding — jsonwebtoken does not check nonce.
        let nonce_ok = data.claims.nonce.as_deref() == Some(expected_nonce);
        anyhow::ensure!(nonce_ok, "id_token nonce mismatch");

        Ok(data.claims)
    }

    /// Build the RP-initiated logout URL (`id_token_hint`, `post_logout_redirect_uri`).
    ///
    /// Returns `None` when the IdP advertises no `end_session_endpoint`.
    #[must_use]
    pub fn rp_logout_url(
        d: &Discovery,
        id_token_hint: &str,
        post_logout_redirect_uri: &str,
    ) -> Option<String> {
        let endpoint = d.end_session_endpoint.as_ref()?;
        let mut url = url::Url::parse(endpoint).ok()?;
        url.query_pairs_mut()
            .append_pair("id_token_hint", id_token_hint)
            .append_pair("post_logout_redirect_uri", post_logout_redirect_uri);
        Some(url.into())
    }
}
