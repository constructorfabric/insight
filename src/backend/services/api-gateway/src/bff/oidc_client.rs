//! Minimal OIDC confidential client.
//!
//! Drives the authorization-code-with-PKCE flow without depending on the
//! `openidconnect` crate, which pins reqwest versions that conflict with
//! the rest of the workspace. We only need:
//!   * discovery doc fetch (cached at module init)
//!   * authorize URL builder (state, nonce, PKCE)
//!   * token exchange (POST `token_endpoint`)
//!   * ID token validation, delegated to `modkit_auth::JwksKeyProvider`
//!     (signature) plus our own `iss`/`aud`/`nonce`/`exp` checks.
//!
//! Anything beyond that — refresh-token use, dynamic client registration,
//! UserInfo, etc. — is out of scope for v1.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use modkit_auth::JwksKeyProvider;
use modkit_auth::traits::KeyProvider;
use rand::RngCore;
use rand::rngs::OsRng;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::bff::errors::BffError;

/// Algorithms accepted on the ID token JWT header. Locked down to the
/// asymmetric set OIDC providers actually use; rejecting `HS*` closes
/// the JWKS-as-HMAC-secret confusion vector, rejecting `none` is
/// belt-and-braces (the JWT crate already rejects it on parse).
const ALLOWED_ALGS: &[&str] = &["RS256", "RS384", "RS512", "ES256", "ES384", "EdDSA"];

/// Clock skew tolerance for `exp` / `nbf` checks on the ID token.
/// Matches the default leeway in upstream JWT libraries.
const CLOCK_SKEW_SECONDS: i64 = 60;

/// Subset of the OIDC discovery document we consume.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct DiscoveryDoc {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    /// Optional: present iff the IdP supports RP-initiated logout.
    /// Used by Phase 2's `/auth/logout`.
    #[serde(default)]
    pub end_session_endpoint: Option<String>,
}

impl DiscoveryDoc {
    /// Fetch the discovery doc. Used once at module init; not on the hot path.
    pub async fn fetch(issuer_url: &str, http_timeout: Duration) -> Result<Self, BffError> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            issuer_url.trim_end_matches('/')
        );
        let client = reqwest::Client::builder()
            .timeout(http_timeout)
            .build()
            .map_err(|e| BffError::Internal(anyhow::anyhow!("reqwest builder: {e}")))?;
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| BffError::Idp(format!("discovery fetch: {e}")))?;
        if !resp.status().is_success() {
            return Err(BffError::Idp(format!(
                "discovery doc returned status {}",
                resp.status()
            )));
        }
        let doc: DiscoveryDoc = resp
            .json()
            .await
            .map_err(|e| BffError::Idp(format!("discovery parse: {e}")))?;
        if doc.issuer.is_empty()
            || doc.authorization_endpoint.is_empty()
            || doc.token_endpoint.is_empty()
            || doc.jwks_uri.is_empty()
        {
            return Err(BffError::Idp(
                "discovery doc missing required fields".into(),
            ));
        }
        Ok(doc)
    }
}

/// PKCE verifier + challenge pair.
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

impl PkcePair {
    /// Generate a fresh verifier (32 random bytes → 43-char base64url) and
    /// derive its S256 challenge.
    #[must_use]
    pub fn generate() -> Self {
        let mut buf = [0u8; 32];
        OsRng.fill_bytes(&mut buf);
        let verifier = URL_SAFE_NO_PAD.encode(buf);
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(h.finalize());
        Self {
            verifier,
            challenge,
        }
    }
}

/// Validated ID token claims surfaced to the rest of the BFF.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IdTokenClaims {
    pub iss: String,
    pub sub: String,
    pub aud: Vec<String>,
    pub exp: i64,
    /// Optional per RFC 7519. We stop short of treating absence as an
    /// error (some IdPs really don't emit it) but never substitute zero
    /// for missing — `None` means "unknown".
    pub iat: Option<i64>,
    pub nbf: Option<i64>,
    pub nonce: Option<String>,
    pub sid: Option<String>,
    /// Authorized party (OIDC Core §3.1.3.7 step 5). Required when `aud`
    /// is multi-valued.
    pub azp: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
}

impl IdTokenClaims {
    fn from_value(value: &serde_json::Value) -> Result<Self, BffError> {
        let v = value
            .as_object()
            .ok_or_else(|| BffError::Idp("id_token claims is not an object".into()))?;
        let str_field = |k: &str| -> Result<String, BffError> {
            v.get(k)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| BffError::Idp(format!("id_token claim `{k}` missing")))
        };
        let opt_str = |k: &str| -> Option<String> {
            v.get(k)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let int_field = |k: &str| -> Result<i64, BffError> {
            v.get(k)
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| BffError::Idp(format!("id_token claim `{k}` missing or not int")))
        };
        let aud = match v.get("aud") {
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect(),
            _ => return Err(BffError::Idp("id_token `aud` claim missing".into())),
        };
        let opt_int = |k: &str| -> Option<i64> { v.get(k).and_then(serde_json::Value::as_i64) };
        Ok(Self {
            iss: str_field("iss")?,
            sub: str_field("sub")?,
            aud,
            exp: int_field("exp")?,
            iat: opt_int("iat"),
            nbf: opt_int("nbf"),
            nonce: opt_str("nonce"),
            sid: opt_str("sid"),
            azp: opt_str("azp"),
            email: opt_str("email"),
            name: opt_str("name").or_else(|| opt_str("preferred_username")),
        })
    }
}

/// Confidential OIDC client.
#[derive(Clone)]
pub struct OidcClient {
    discovery: DiscoveryDoc,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    expected_audience: String,
    scopes: Vec<String>,
    http: reqwest::Client,
    jwks: Arc<JwksKeyProvider>,
}

impl OidcClient {
    /// Build a new client. Performs discovery + initial JWKS fetch.
    pub async fn new(
        issuer_url: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        scopes: Vec<String>,
        audience: Option<&str>,
    ) -> Result<Self, BffError> {
        let discovery = DiscoveryDoc::fetch(issuer_url, Duration::from_secs(10)).await?;
        let jwks = Arc::new(
            JwksKeyProvider::new(discovery.jwks_uri.clone())
                .map_err(|e| BffError::Internal(anyhow::anyhow!("jwks provider: {e}")))?
                .with_refresh_interval(Duration::from_secs(3600)),
        );
        // Initial fetch — not fatal if it fails (the IdP may be briefly
        // unreachable at boot); on-demand refresh will retry.
        if let Err(e) = jwks.refresh_keys().await {
            tracing::warn!(error = %e, "initial JWKS fetch failed; will retry on demand");
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| BffError::Internal(anyhow::anyhow!("reqwest builder: {e}")))?;

        Ok(Self {
            discovery,
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            redirect_uri: redirect_uri.to_owned(),
            expected_audience: audience.unwrap_or(client_id).to_owned(),
            scopes,
            http,
            jwks,
        })
    }

    /// Phase-2 access for `/auth/logout` (id_token_hint, end_session_url).
    #[must_use]
    #[allow(dead_code)]
    pub fn discovery(&self) -> &DiscoveryDoc {
        &self.discovery
    }

    /// Build the authorize URL the browser is redirected to.
    pub fn authorize_url(
        &self,
        state: &str,
        nonce: &str,
        pkce_challenge: &str,
    ) -> Result<String, BffError> {
        let mut url = url::Url::parse(&self.discovery.authorization_endpoint)
            .map_err(|e| BffError::Internal(anyhow::anyhow!("invalid auth endpoint: {e}")))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("scope", &self.scopes.join(" "))
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", pkce_challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url.to_string())
    }

    /// Exchange a callback `code` for tokens. Returns the raw `id_token`
    /// (so we can keep it for `id_token_hint` later) plus the validated
    /// claims.
    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        expected_nonce: &str,
    ) -> Result<TokenExchange, BffError> {
        let resp = self
            .http
            .post(&self.discovery.token_endpoint)
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("code_verifier", verifier),
                // client_id repeated in body for IdPs that demand it even
                // alongside Basic auth (Keycloak, some Auth0 setups).
                ("client_id", self.client_id.as_str()),
            ])
            .send()
            .await
            .map_err(|e| BffError::Idp(format!("token endpoint: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            // Pull `error` / `error_description` out of a JSON body if the
            // IdP supplied them — that's the actionable signal. Don't echo
            // the raw body into the response or logs (could carry user
            // input or partial grant data).
            let body_text = resp.text().await.unwrap_or_default();
            let parsed = serde_json::from_str::<TokenError>(&body_text).ok();
            let summary = parsed.as_ref().map_or_else(
                || status.to_string(),
                |e| {
                    format!(
                        "{} ({})",
                        e.error,
                        e.error_description.as_deref().unwrap_or("")
                    )
                },
            );
            tracing::warn!(
                status = %status,
                error = %parsed.as_ref().map_or("", |e| e.error.as_str()),
                "token endpoint non-success",
            );
            return Err(BffError::Idp(format!("token endpoint: {summary}")));
        }

        let body: TokenResponse = resp
            .json()
            .await
            .map_err(|e| BffError::Idp(format!("token response parse: {e}")))?;

        let id_token = body
            .id_token
            .ok_or_else(|| BffError::Idp("token response missing id_token".into()))?;

        // Algorithm allow-list — guard against alg-confusion / `none`. We
        // peek at the JWT header before delegating signature verification
        // to the JWKS provider, since the provider does not enforce an
        // allow-list itself.
        verify_alg_against_allowlist(&id_token)?;

        // Signature check via JWKS provider. The provider validates the
        // signature only — `exp`/`nbf`/`iss`/`aud`/`nonce` are this BFF's
        // responsibility (see `validate_id_token_claims`).
        let (_header, claims_value) = self
            .jwks
            .validate_and_decode(&id_token)
            .await
            .map_err(|e| BffError::Idp(format!("id_token validate: {e}")))?;

        let claims = IdTokenClaims::from_value(&claims_value)?;
        let now = unix_now();
        validate_id_token_claims(
            &claims,
            &self.discovery.issuer,
            &self.expected_audience,
            &self.client_id,
            expected_nonce,
            now,
        )?;

        Ok(TokenExchange { id_token, claims })
    }

    /// Build the RP-initiated logout URL. Returns `None` if the IdP did
    /// not advertise an `end_session_endpoint`. Phase 2's `/auth/logout`.
    #[must_use]
    #[allow(dead_code)]
    pub fn end_session_url(
        &self,
        id_token_hint: &str,
        post_logout_redirect: &str,
    ) -> Option<String> {
        let endpoint = self.discovery.end_session_endpoint.as_ref()?;
        let mut url = url::Url::parse(endpoint).ok()?;
        url.query_pairs_mut()
            .append_pair("id_token_hint", id_token_hint)
            .append_pair("post_logout_redirect_uri", post_logout_redirect)
            .append_pair("client_id", &self.client_id);
        Some(url.to_string())
    }
}

/// Successful token-exchange result.
pub struct TokenExchange {
    pub id_token: String,
    pub claims: IdTokenClaims,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    id_token: Option<String>,
}

/// OAuth 2.0 / OIDC error response shape (RFC 6749 §5.2).
#[derive(Debug, Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

fn unix_now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
    .unwrap_or(0)
}

/// Decode the JWT header without verifying signature, and reject any
/// algorithm not in `ALLOWED_ALGS`. This closes the alg-confusion family:
/// `none`, `HS256`-with-RSA-pubkey-as-shared-secret, downgrade games.
fn verify_alg_against_allowlist(token: &str) -> Result<(), BffError> {
    // header is the first dot-separated segment, base64url-encoded JSON.
    let header_b64 = token
        .split('.')
        .next()
        .ok_or_else(|| BffError::Idp("id_token has no header".into()))?;
    let raw = URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|e| BffError::Idp(format!("id_token header decode: {e}")))?;
    let header: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| BffError::Idp(format!("id_token header parse: {e}")))?;
    let alg = header
        .get("alg")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| BffError::Idp("id_token header missing alg".into()))?;
    if !ALLOWED_ALGS.contains(&alg) {
        return Err(BffError::Idp(format!(
            "id_token alg `{alg}` not in allow-list"
        )));
    }
    Ok(())
}

/// Validate iss/aud/nonce/exp/nbf/azp on a parsed `IdTokenClaims`.
///
/// Pulled out of `exchange_code` so it's directly testable with a fake
/// `IdTokenClaims` and a fixed `now`, no IdP needed.
///
/// Per OIDC Core §3.1.3.7:
///   * `iss` must match the configured issuer exactly.
///   * `aud` must contain the expected audience.
///   * If `aud` is multi-valued AND `azp` is present, `azp` must equal
///     the client_id.
///   * `nonce` must match what the BFF stored at /auth/login time.
///   * `exp` must be in the future (with clock skew leeway).
///   * `nbf`, if present, must not be in the future (with leeway).
fn validate_id_token_claims(
    claims: &IdTokenClaims,
    expected_issuer: &str,
    expected_audience: &str,
    client_id: &str,
    expected_nonce: &str,
    now: i64,
) -> Result<(), BffError> {
    if claims.iss != expected_issuer {
        return Err(BffError::Idp("id_token iss mismatch".into()));
    }
    if !claims.aud.iter().any(|a| a == expected_audience) {
        return Err(BffError::Idp("id_token aud mismatch".into()));
    }
    // §3.1.3.7 step 5: if multiple audiences, azp MUST be present and equal client_id.
    if claims.aud.len() > 1 {
        match claims.azp.as_deref() {
            Some(azp) if azp == client_id => {}
            Some(_) => {
                return Err(BffError::Idp("id_token azp mismatch".into()));
            }
            None => {
                return Err(BffError::Idp(
                    "id_token has multiple audiences but no azp".into(),
                ));
            }
        }
    }
    match claims.nonce.as_deref() {
        Some(n) if n == expected_nonce => {}
        _ => return Err(BffError::Idp("id_token nonce mismatch".into())),
    }
    if claims.exp <= now.saturating_sub(CLOCK_SKEW_SECONDS) {
        return Err(BffError::Idp("id_token expired".into()));
    }
    if let Some(nbf) = claims.nbf
        && nbf > now.saturating_add(CLOCK_SKEW_SECONDS)
    {
        return Err(BffError::Idp("id_token not yet valid (nbf)".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_pair_uses_url_safe_no_pad() {
        let p = PkcePair::generate();
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(p.challenge.len(), 43);
        assert!(!p.verifier.contains('='));
        assert!(!p.challenge.contains('='));
        assert!(!p.challenge.contains('+'));
        assert!(!p.challenge.contains('/'));
    }

    #[test]
    fn pkce_pair_challenge_matches_s256_of_verifier() {
        let p = PkcePair::generate();
        let mut h = Sha256::new();
        h.update(p.verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(h.finalize());
        assert_eq!(p.challenge, expected);
    }

    #[test]
    fn pkce_pair_verifier_is_random() {
        let a = PkcePair::generate();
        let b = PkcePair::generate();
        assert_ne!(a.verifier, b.verifier);
    }

    #[test]
    fn id_token_claims_parse_minimal() {
        let v = serde_json::json!({
            "iss": "https://idp/",
            "sub": "sub-1",
            "aud": "client-id",
            "exp": 1_700_000_000_i64,
            "nonce": "abc",
        });
        let c = IdTokenClaims::from_value(&v).expect("parse");
        assert_eq!(c.iss, "https://idp/");
        assert_eq!(c.sub, "sub-1");
        assert_eq!(c.aud, vec!["client-id".to_owned()]);
        assert_eq!(c.exp, 1_700_000_000);
        assert_eq!(c.nonce.as_deref(), Some("abc"));
    }

    #[test]
    fn id_token_claims_parse_aud_array() {
        let v = serde_json::json!({
            "iss": "i",
            "sub": "s",
            "aud": ["a", "b"],
            "exp": 1_i64,
        });
        let c = IdTokenClaims::from_value(&v).expect("parse");
        assert_eq!(c.aud, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn id_token_claims_reject_missing_iss() {
        let v = serde_json::json!({"sub": "s", "aud": "x", "exp": 1});
        assert!(IdTokenClaims::from_value(&v).is_err());
    }

    #[test]
    fn id_token_claims_reject_missing_aud() {
        let v = serde_json::json!({"iss": "i", "sub": "s", "exp": 1});
        assert!(IdTokenClaims::from_value(&v).is_err());
    }

    #[test]
    fn id_token_claims_extract_email_and_name() {
        let v = serde_json::json!({
            "iss": "i", "sub": "s", "aud": "x", "exp": 1,
            "email": "a@b.com", "name": "Alice",
        });
        let c = IdTokenClaims::from_value(&v).expect("parse");
        assert_eq!(c.email.as_deref(), Some("a@b.com"));
        assert_eq!(c.name.as_deref(), Some("Alice"));
    }

    #[test]
    fn id_token_claims_falls_back_to_preferred_username() {
        let v = serde_json::json!({
            "iss": "i", "sub": "s", "aud": "x", "exp": 1,
            "preferred_username": "alice",
        });
        let c = IdTokenClaims::from_value(&v).expect("parse");
        assert_eq!(c.name.as_deref(), Some("alice"));
    }

    #[test]
    fn id_token_claims_iat_is_none_when_missing() {
        let v = serde_json::json!({"iss": "i", "sub": "s", "aud": "x", "exp": 1});
        let c = IdTokenClaims::from_value(&v).expect("parse");
        assert_eq!(c.iat, None);
        assert_eq!(c.nbf, None);
        assert_eq!(c.azp, None);
    }

    fn good_claims(now: i64, nonce: &str) -> IdTokenClaims {
        IdTokenClaims {
            iss: "https://idp/".into(),
            sub: "sub".into(),
            aud: vec!["client-id".into()],
            exp: now + 300,
            iat: Some(now),
            nbf: None,
            nonce: Some(nonce.into()),
            sid: None,
            azp: None,
            email: Some("a@b.com".into()),
            name: Some("Alice".into()),
        }
    }

    #[test]
    fn validate_claims_accepts_good_token() {
        let c = good_claims(1_000_000, "n");
        validate_id_token_claims(&c, "https://idp/", "client-id", "client-id", "n", 1_000_000)
            .expect("ok");
    }

    #[test]
    fn validate_claims_rejects_iss_mismatch() {
        let c = good_claims(1_000_000, "n");
        let r = validate_id_token_claims(
            &c,
            "https://other/",
            "client-id",
            "client-id",
            "n",
            1_000_000,
        );
        assert!(r.is_err());
    }

    #[test]
    fn validate_claims_rejects_aud_mismatch() {
        let c = good_claims(1_000_000, "n");
        let r =
            validate_id_token_claims(&c, "https://idp/", "other-aud", "client-id", "n", 1_000_000);
        assert!(r.is_err());
    }

    #[test]
    fn validate_claims_rejects_nonce_mismatch() {
        let c = good_claims(1_000_000, "n");
        let r = validate_id_token_claims(
            &c,
            "https://idp/",
            "client-id",
            "client-id",
            "wrong",
            1_000_000,
        );
        assert!(r.is_err());
    }

    #[test]
    fn validate_claims_rejects_missing_nonce() {
        let mut c = good_claims(1_000_000, "n");
        c.nonce = None;
        let r =
            validate_id_token_claims(&c, "https://idp/", "client-id", "client-id", "n", 1_000_000);
        assert!(r.is_err());
    }

    #[test]
    fn validate_claims_rejects_expired_token() {
        let c = good_claims(1_000_000, "n");
        // now is past exp + leeway
        let r =
            validate_id_token_claims(&c, "https://idp/", "client-id", "client-id", "n", 1_000_400);
        assert!(r.is_err());
    }

    #[test]
    fn validate_claims_accepts_token_within_clock_skew_after_exp() {
        let mut c = good_claims(1_000_000, "n");
        c.exp = 1_000_000;
        // now is 30s past exp — within 60s leeway
        let r =
            validate_id_token_claims(&c, "https://idp/", "client-id", "client-id", "n", 1_000_030);
        assert!(r.is_ok(), "30s past exp should be in leeway");
    }

    #[test]
    fn validate_claims_rejects_future_nbf() {
        let mut c = good_claims(1_000_000, "n");
        c.nbf = Some(1_000_500); // 500s in future, past leeway
        let r =
            validate_id_token_claims(&c, "https://idp/", "client-id", "client-id", "n", 1_000_000);
        assert!(r.is_err());
    }

    #[test]
    fn validate_claims_accepts_nbf_within_leeway() {
        let mut c = good_claims(1_000_000, "n");
        c.nbf = Some(1_000_030); // 30s in future — within 60s leeway
        let r =
            validate_id_token_claims(&c, "https://idp/", "client-id", "client-id", "n", 1_000_000);
        assert!(r.is_ok());
    }

    #[test]
    fn validate_claims_requires_azp_when_aud_is_multi() {
        let mut c = good_claims(1_000_000, "n");
        c.aud = vec!["client-id".into(), "other-aud".into()];
        let r =
            validate_id_token_claims(&c, "https://idp/", "client-id", "client-id", "n", 1_000_000);
        assert!(r.is_err(), "multi-aud without azp must fail");
    }

    #[test]
    fn validate_claims_accepts_multi_aud_with_correct_azp() {
        let mut c = good_claims(1_000_000, "n");
        c.aud = vec!["client-id".into(), "other-aud".into()];
        c.azp = Some("client-id".into());
        validate_id_token_claims(&c, "https://idp/", "client-id", "client-id", "n", 1_000_000)
            .expect("ok");
    }

    #[test]
    fn validate_claims_rejects_multi_aud_with_wrong_azp() {
        let mut c = good_claims(1_000_000, "n");
        c.aud = vec!["client-id".into(), "other-aud".into()];
        c.azp = Some("attacker".into());
        let r =
            validate_id_token_claims(&c, "https://idp/", "client-id", "client-id", "n", 1_000_000);
        assert!(r.is_err());
    }

    fn header_b64(alg: &str) -> String {
        let json = serde_json::json!({"alg": alg, "typ": "JWT"});
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json).expect("ser"))
    }

    fn jwt_with_alg(alg: &str) -> String {
        let header = header_b64(alg);
        let payload = URL_SAFE_NO_PAD.encode(b"{}");
        let sig = URL_SAFE_NO_PAD.encode(b"x");
        format!("{header}.{payload}.{sig}")
    }

    #[test]
    fn alg_allowlist_accepts_rs256() {
        verify_alg_against_allowlist(&jwt_with_alg("RS256")).expect("ok");
    }

    #[test]
    fn alg_allowlist_rejects_none() {
        let r = verify_alg_against_allowlist(&jwt_with_alg("none"));
        assert!(r.is_err());
    }

    #[test]
    fn alg_allowlist_rejects_hs256() {
        // HMAC family is what enables key-confusion; must reject.
        let r = verify_alg_against_allowlist(&jwt_with_alg("HS256"));
        assert!(r.is_err());
    }

    #[test]
    fn alg_allowlist_rejects_unknown() {
        let r = verify_alg_against_allowlist(&jwt_with_alg("PS256")); // PS family not in list
        assert!(r.is_err());
    }

    #[test]
    fn alg_allowlist_rejects_missing_alg_field() {
        // header without alg
        let header = URL_SAFE_NO_PAD.encode(b"{\"typ\":\"JWT\"}");
        let payload = URL_SAFE_NO_PAD.encode(b"{}");
        let sig = URL_SAFE_NO_PAD.encode(b"x");
        let r = verify_alg_against_allowlist(&format!("{header}.{payload}.{sig}"));
        assert!(r.is_err());
    }
}
