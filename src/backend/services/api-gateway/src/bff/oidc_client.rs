//! OIDC confidential client, backed by the `openidconnect` crate.
//!
//! Thin adapter around `openidconnect 4`: discovery, authorize-URL
//! builder, code-with-PKCE exchange, and ID-token verification
//! (signature + `iss`/`aud`/`nonce`/`exp`/`nbf` + alg allow-list) all
//! live in the upstream crate. The local surface mirrors what the
//! previous hand-rolled module exposed so callers
//! (`handlers::login`, `handlers::callback`, `module`) didn't change.
//!
//! Anything beyond the authorization-code-with-PKCE flow — refresh
//! tokens, dynamic client registration, UserInfo — is out of scope
//! for Phase 1. RP-initiated logout will be wired through the crate's
//! `ProviderMetadataWithLogout` extension when the `/auth/logout`
//! handler lands.

use std::sync::Arc;
use std::time::Duration;

use openidconnect::core::{
    CoreAuthDisplay, CoreAuthPrompt, CoreAuthenticationFlow, CoreErrorResponseType,
    CoreGenderClaim, CoreJsonWebKey, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreProviderMetadata, CoreRevocableToken, CoreTokenIntrospectionResponse, CoreTokenType,
};
use openidconnect::{
    AdditionalClaims, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EmptyExtraTokenFields,
    EndpointMaybeSet, EndpointNotSet, EndpointSet, IdTokenFields, IssuerUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RevocationErrorResponseType, Scope,
    StandardErrorResponse, StandardTokenResponse, TokenResponse, reqwest,
};
use serde::{Deserialize, Serialize};

use crate::bff::errors::BffError;

/// Asymmetric signing algorithms the BFF accepts on ID tokens. HMAC
/// (`HS*`) is excluded to close the JWKS-as-shared-secret confusion
/// vector; `none` is excluded for the obvious reason. Enforced via the
/// verifier's `set_allowed_algs` knob inside `exchange_code`.
fn allowed_signing_algs() -> Vec<CoreJwsSigningAlgorithm> {
    vec![
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha384,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha512,
        CoreJwsSigningAlgorithm::EcdsaP256Sha256,
        CoreJwsSigningAlgorithm::EcdsaP384Sha384,
        CoreJwsSigningAlgorithm::EdDsa,
    ]
}

/// Extra ID-token claims the BFF cares about beyond the OIDC standard
/// set — namely `sid`, used by back-channel logout to map IdP sessions
/// to local ones.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BffAdditionalClaims {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
}

impl AdditionalClaims for BffAdditionalClaims {}

type BffIdTokenFields = IdTokenFields<
    BffAdditionalClaims,
    EmptyExtraTokenFields,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;

type BffTokenResponse = StandardTokenResponse<BffIdTokenFields, CoreTokenType>;

/// Concrete client type after discovery + redirect URI are set. The
/// typestate parameters mirror what `from_provider_metadata` produces:
/// auth URL is always supplied by discovery (`EndpointSet`), token and
/// userinfo URLs may or may not be (`EndpointMaybeSet`), and the rest
/// stay `NotSet` since the BFF never speaks device-flow, introspection,
/// or revocation.
type BffOidcClient = openidconnect::Client<
    BffAdditionalClaims,
    CoreAuthDisplay,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJsonWebKey,
    CoreAuthPrompt,
    StandardErrorResponse<CoreErrorResponseType>,
    BffTokenResponse,
    CoreTokenIntrospectionResponse,
    CoreRevocableToken,
    StandardErrorResponse<RevocationErrorResponseType>,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// PKCE verifier. The crate deliberately makes
/// `PkceCodeChallenge` non-`Clone` and constructible only from a
/// verifier, so the BFF persists the verifier (in
/// `bff:login_state:{state}`) and re-derives the challenge inside
/// `authorize_url` each time. Two-field shape is gone; the public
/// field is now `verifier` only.
pub struct PkcePair {
    pub verifier: String,
}

impl PkcePair {
    #[must_use]
    pub fn generate() -> Self {
        let (_challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        Self {
            verifier: verifier.secret().clone(),
        }
    }
}

/// Validated ID token claims surfaced to the rest of the BFF.
///
/// Projection over `openidconnect::IdTokenClaims<BffAdditionalClaims,
/// CoreGenderClaim>`: the crate's claims type is verbose (locale-
/// aware names, generic over key/alg/claim types) and verifier-bound,
/// so we collapse it to the small set of values the handlers actually
/// consume. Every value here has already cleared the crate's verifier
/// (signature + iss + aud + nonce + exp/nbf + alg allow-list).
#[derive(Debug, Clone)]
pub struct IdTokenClaims {
    pub iss: String,
    pub sub: String,
    pub sid: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
}

/// Successful token-exchange result.
pub struct TokenExchange {
    pub id_token: String,
    pub claims: IdTokenClaims,
}

/// Confidential OIDC client.
#[derive(Clone)]
pub struct OidcClient {
    client: Arc<BffOidcClient>,
    http: reqwest::Client,
    metadata: CoreProviderMetadata,
    /// Optional explicit audience override. `None` falls back to the
    /// crate verifier's default (matches `client_id`).
    expected_audience: Option<String>,
    scopes: Vec<Scope>,
}

impl OidcClient {
    /// Build a new client. Performs discovery; the JWKS arrives as
    /// part of the provider metadata and stays cached inside the
    /// crate's client for the duration of the process.
    pub async fn new(
        issuer_url: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        scopes: Vec<String>,
        audience: Option<&str>,
    ) -> Result<Self, BffError> {
        let http = reqwest::ClientBuilder::new()
            .timeout(Duration::from_secs(15))
            // Mitigation called out by the openidconnect docs: following
            // 3xx redirects on token/userinfo/jwks fetches opens SSRF
            // against the provider's hostname.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| BffError::Internal(anyhow::anyhow!("reqwest builder: {e}")))?;

        let issuer = IssuerUrl::new(issuer_url.to_owned())
            .map_err(|e| BffError::Idp(format!("invalid issuer_url: {e}")))?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|e| BffError::Idp(format!("discovery failed: {e}")))?;

        let redirect = RedirectUrl::new(redirect_uri.to_owned())
            .map_err(|e| BffError::Internal(anyhow::anyhow!("invalid redirect_uri: {e}")))?;

        let client: BffOidcClient = openidconnect::Client::from_provider_metadata(
            metadata.clone(),
            ClientId::new(client_id.to_owned()),
            Some(ClientSecret::new(client_secret.to_owned())),
        )
        .set_redirect_uri(redirect);

        Ok(Self {
            client: Arc::new(client),
            http,
            metadata,
            expected_audience: audience.map(str::to_owned),
            scopes: scopes.into_iter().map(Scope::new).collect(),
        })
    }

    /// Phase-2 access for `/auth/logout` (id_token_hint, end_session_url).
    #[must_use]
    #[allow(dead_code)]
    pub fn metadata(&self) -> &CoreProviderMetadata {
        &self.metadata
    }

    /// Build the authorize URL the browser is redirected to. State and
    /// nonce are generated by the BFF (so they can be persisted
    /// alongside the PKCE verifier in Redis) and supplied here; the
    /// crate wraps them in `CsrfToken` / `Nonce` newtypes and stitches
    /// them onto the URL. The PKCE challenge is derived from the
    /// verifier per `from_code_verifier_sha256`.
    #[must_use]
    pub fn authorize_url(&self, state: &str, nonce: &str, pkce_verifier: &str) -> String {
        let state_owned = state.to_owned();
        let nonce_owned = nonce.to_owned();
        let verifier = PkceCodeVerifier::new(pkce_verifier.to_owned());
        let challenge = PkceCodeChallenge::from_code_verifier_sha256(&verifier);

        let mut builder = self.client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            move || CsrfToken::new(state_owned.clone()),
            move || Nonce::new(nonce_owned.clone()),
        );
        for scope in &self.scopes {
            builder = builder.add_scope(scope.clone());
        }
        let (url, _csrf, _nonce) = builder.set_pkce_challenge(challenge).url();
        url.to_string()
    }

    /// Exchange a callback `code` for tokens. Returns the raw
    /// `id_token` (so callers can keep it for `id_token_hint` in
    /// RP-initiated logout) plus the verified claims.
    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        expected_nonce: &str,
    ) -> Result<TokenExchange, BffError> {
        let token_response = self
            .client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .map_err(|e| BffError::Idp(format!("exchange_code build: {e}")))?
            .set_pkce_verifier(PkceCodeVerifier::new(verifier.to_owned()))
            .request_async(&self.http)
            .await
            .map_err(|e| BffError::Idp(format!("token endpoint: {e}")))?;

        let id_token = token_response
            .id_token()
            .ok_or_else(|| BffError::Idp("token response missing id_token".into()))?
            .clone();

        let mut verifier_cfg = self
            .client
            .id_token_verifier()
            .set_allowed_algs(allowed_signing_algs());
        if let Some(aud) = self.expected_audience.clone() {
            verifier_cfg = verifier_cfg.set_other_audience_verifier_fn(move |a| a.as_str() == aud);
        }

        let nonce = Nonce::new(expected_nonce.to_owned());
        let claims = id_token
            .claims(&verifier_cfg, &nonce)
            .map_err(|e| BffError::Idp(format!("id_token validate: {e}")))?;

        let projected = project_claims(claims);

        Ok(TokenExchange {
            id_token: id_token.to_string(),
            claims: projected,
        })
    }
}

fn project_claims(
    c: &openidconnect::IdTokenClaims<BffAdditionalClaims, CoreGenderClaim>,
) -> IdTokenClaims {
    // Prefer the unlocalized `name`, fall back to the first localized
    // entry, then to `preferred_username`. The session record only ever
    // stores one display string, so flattening here keeps callers
    // unaware of `LocalizedClaim`.
    let name = c
        .name()
        .and_then(|lc| {
            lc.get(None)
                .map(|n| n.as_str().to_owned())
                .or_else(|| lc.iter().next().map(|(_, n)| n.as_str().to_owned()))
        })
        .or_else(|| c.preferred_username().map(|p| p.as_str().to_owned()));

    IdTokenClaims {
        iss: c.issuer().as_str().to_owned(),
        sub: c.subject().as_str().to_owned(),
        sid: c.additional_claims().sid.clone(),
        email: c.email().map(|e| e.as_str().to_owned()),
        name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_is_url_safe_no_pad() {
        let p = PkcePair::generate();
        assert!(!p.verifier.is_empty());
        assert!(!p.verifier.contains('='));
        assert!(!p.verifier.contains('+'));
        assert!(!p.verifier.contains('/'));
    }

    #[test]
    fn pkce_verifier_is_random() {
        let a = PkcePair::generate();
        let b = PkcePair::generate();
        assert_ne!(a.verifier, b.verifier);
    }

    #[test]
    fn additional_claims_round_trips_sid() {
        let json = serde_json::json!({"sid": "abc123"});
        let parsed: BffAdditionalClaims = serde_json::from_value(json).expect("parse");
        assert_eq!(parsed.sid.as_deref(), Some("abc123"));
        let rendered = serde_json::to_value(&parsed).expect("ser");
        assert_eq!(rendered, serde_json::json!({"sid": "abc123"}));
    }

    #[test]
    fn additional_claims_omits_sid_when_absent() {
        let parsed: BffAdditionalClaims =
            serde_json::from_value(serde_json::json!({})).expect("parse");
        assert!(parsed.sid.is_none());
        let rendered = serde_json::to_value(&parsed).expect("ser");
        assert_eq!(rendered, serde_json::json!({}));
    }
}
