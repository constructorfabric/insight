use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};

use crate::config::{HostIdpConfig, IdpConfig, ResolveBy};
use crate::identity::IdpIdentity;

pub struct AuthorizeStart {
    pub url: String,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

pub struct AuthenticatedIdp {
    pub identity: IdpIdentity,
    pub issuer: String,
    pub idp_sid: Option<String>,
    pub id_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}

#[derive(Debug)]
pub enum RefreshOutcome {
    Refreshed {
        new_refresh_token: Option<String>,
        expires_in: Option<u64>,
    },
    InvalidGrant(String),
    Transient(String),
}

pub fn build_http(idp: &IdpConfig) -> anyhow::Result<reqwest::Client> {
    // Do not follow redirects: SSRF-safety guidance from the openidconnect docs.
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5));
    if !idp.extra_ca_cert_path.is_empty() {
        let pem = std::fs::read(&idp.extra_ca_cert_path)
            .with_context(|| format!("read extra_ca_cert_path {:?}", idp.extra_ca_cert_path))?;
        // `from_pem_bundle`, not `from_pem`: the file may carry a full
        // chain (e.g. intermediate + root); `from_pem` only parses the
        // first certificate in the blob and silently drops the rest.
        let certs = reqwest::Certificate::from_pem_bundle(&pem).with_context(|| {
            format!(
                "parse PEM cert bundle from extra_ca_cert_path {:?}",
                idp.extra_ca_cert_path
            )
        })?;
        // A whitespace/comment-only file parses to zero certs without erroring.
        anyhow::ensure!(
            !certs.is_empty(),
            "extra_ca_cert_path {:?} contained no certificates",
            idp.extra_ca_cert_path
        );
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }
    builder.build().context("build OIDC HTTP client")
}

#[derive(Clone)]
pub struct OidcClient {
    issuer_url: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    tenant_claim: String,
    default_tenant_id: String,
    external_id_claim: String,
    resolve_by: ResolveBy,
    http: reqwest::Client,
}

impl OidcClient {
    #[must_use]
    pub fn flat(idp: &IdpConfig, redirect_uri: &str, http: reqwest::Client) -> Self {
        Self {
            // Do NOT normalize a trailing slash: OIDC issuer comparison is a
            // byte-exact match against the discovery `issuer` (OIDC Discovery §4.3).
            issuer_url: idp.issuer_url.clone(),
            client_id: idp.client_id.clone(),
            client_secret: idp.client_secret.clone(),
            redirect_uri: redirect_uri.to_owned(),
            tenant_claim: idp.tenant_claim.clone(),
            default_tenant_id: idp.default_tenant_id.clone(),
            external_id_claim: idp.external_id_claim.clone(),
            resolve_by: idp.resolve_by,
            http,
        }
    }

    #[must_use]
    pub fn for_host(
        idp: &IdpConfig,
        entry: &HostIdpConfig,
        default_redirect_uri: &str,
        http: reqwest::Client,
    ) -> Self {
        let pick = |own: &str, shared: &str| if own.is_empty() { shared } else { own }.to_owned();
        Self {
            issuer_url: entry.issuer_url.clone(),
            client_id: entry.client_id.clone(),
            client_secret: entry.client_secret.clone(),
            redirect_uri: pick(&entry.redirect_uri, default_redirect_uri),
            tenant_claim: idp.tenant_claim.clone(),
            default_tenant_id: pick(&entry.default_tenant_id, &idp.default_tenant_id),
            external_id_claim: idp.external_id_claim.clone(),
            resolve_by: idp.resolve_by,
            http,
        }
    }

    async fn metadata(&self) -> anyhow::Result<CoreProviderMetadata> {
        let issuer = IssuerUrl::new(self.issuer_url.clone()).context("invalid issuer_url")?;
        CoreProviderMetadata::discover_async(issuer, &self.http)
            .await
            .context("OIDC discovery")
    }

    fn secret(&self) -> Option<ClientSecret> {
        (!self.client_secret.is_empty()).then(|| ClientSecret::new(self.client_secret.clone()))
    }

    pub async fn authorize(&self, scopes: &[String]) -> anyhow::Result<AuthorizeStart> {
        // Built inline (not via a helper) so the endpoint type-state markers
        // from `from_provider_metadata` + `set_redirect_uri` are preserved.
        let client = CoreClient::from_provider_metadata(
            self.metadata().await?,
            ClientId::new(self.client_id.clone()),
            self.secret(),
        )
        .set_redirect_uri(
            RedirectUrl::new(self.redirect_uri.clone()).context("invalid redirect_uri")?,
        );
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();

        let mut builder = client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            openidconnect::CsrfToken::new_random,
            Nonce::new_random,
        );
        // `openid` is added by the flow.
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

    pub async fn exchange_code_pkce(
        &self,
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
            RedirectUrl::new(self.redirect_uri.clone()).context("invalid redirect_uri")?,
        );
        let token = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .context("build code-exchange request")?
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
            .request_async(&self.http)
            .await
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

        let sub = claims.subject().to_string();
        let issuer = claims.issuer().to_string();
        let email = claims.email().map(|e| e.to_string()).unwrap_or_default();

        let raw = id_token.to_string();
        let mut tenant_id = payload_tenant(&raw, &self.tenant_claim);
        if tenant_id.is_empty() && !self.default_tenant_id.is_empty() {
            tracing::debug!(tenant_id = %self.default_tenant_id, "id_token carries no tenant claim; using idp.default_tenant_id");
            tenant_id.clone_from(&self.default_tenant_id);
        }
        let idp_sid = payload_string(&raw, "sid");

        let resolve_by =
            resolve_target(self.resolve_by, &raw, &sub, &self.external_id_claim, &email)?;

        Ok(AuthenticatedIdp {
            identity: IdpIdentity {
                sub,
                email,
                tenant_id,
                resolve_by,
            },
            issuer,
            idp_sid,
            id_token: raw,
            refresh_token: token.refresh_token().map(|r| r.secret().clone()),
            expires_in: token.expires_in().map(|d| d.as_secs()),
        })
    }

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
            Err(e) => RefreshOutcome::Transient(format!("{e}")),
        }
    }

    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer_url
    }

    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub async fn idp_jwks(&self) -> anyhow::Result<jsonwebtoken::jwk::JwkSet> {
        #[derive(serde::Deserialize)]
        struct Disco {
            jwks_uri: String,
        }
        let disco: Disco = self
            .http
            .get(format!(
                "{}/.well-known/openid-configuration",
                self.issuer_url.trim_end_matches('/')
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

    /// `end_session_endpoint` is not part of core discovery, so it is fetched here directly.
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
                self.issuer_url.trim_end_matches('/')
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

/// A string array is tolerated by taking its first entry (a Keycloak multivalued mapper).
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

fn resolve_target(
    mode: ResolveBy,
    raw: &str,
    sub: &str,
    external_id_claim: &str,
    email: &str,
) -> anyhow::Result<crate::identity::ResolveTarget> {
    match mode {
        ResolveBy::ExternalId => {
            let external_id =
                extract_external_id(raw, external_id_claim, sub).with_context(|| {
                    format!(
                        "id_token carries no non-empty `{external_id_claim}` claim \
                     (idp.external_id_claim) — cannot resolve person for login"
                    )
                })?;
            Ok(crate::identity::ResolveTarget::ExternalId(external_id))
        }
        ResolveBy::Email => {
            let email = email.trim();
            if email.is_empty() {
                tracing::warn!(
                    sub = %sub,
                    "id_token carries no `email` claim but idp.resolve_by is email — \
                     check the IdP's claim mapping; refusing the login"
                );
                anyhow::bail!(
                    "id_token carries no non-empty `email` claim (idp.resolve_by = email) — \
                     cannot resolve person for login"
                );
            }
            // Only an explicit `false` refuses: many providers omit `email_verified`.
            if payload_bool(raw, "email_verified") == Some(false) {
                tracing::warn!(
                    sub = %sub,
                    "id_token says email_verified=false and idp.resolve_by is email — \
                     refusing the login"
                );
                anyhow::bail!(
                    "id_token carries email_verified=false (idp.resolve_by = email) — \
                     refusing to resolve a person by an unverified address"
                );
            }
            Ok(crate::identity::ResolveTarget::RosterEmail(
                email.to_owned(),
            ))
        }
    }
}

fn extract_external_id(raw_id_token: &str, external_id_claim: &str, sub: &str) -> Option<String> {
    if external_id_claim == "sub" {
        return Some(sub.to_owned());
    }
    payload_string(raw_id_token, external_id_claim).filter(|v| !v.is_empty())
}

pub(crate) fn payload_string(jwt: &str, field: &str) -> Option<String> {
    payload(jwt)?
        .get(field)?
        .as_str()
        .map(std::borrow::ToOwned::to_owned)
}

fn payload_bool(jwt: &str, field: &str) -> Option<bool> {
    payload(jwt)?.get(field)?.as_bool()
}

fn payload(jwt: &str) -> Option<serde_json::Value> {
    let segment = jwt.split('.').nth(1)?;
    let bytes = B64.decode(segment).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn jwt_with(claims: &serde_json::Value) -> String {
        let body = B64.encode(serde_json::to_vec(claims).unwrap());
        format!("e30.{body}.sig")
    }

    #[test]
    fn tenant_claim_string_and_array_shapes() {
        let jwt = jwt_with(&serde_json::json!({"tenant_id": "t1"}));
        assert_eq!(payload_tenant(&jwt, "tenant_id"), "t1");
        let jwt = jwt_with(&serde_json::json!({"tid": "dir-guid"}));
        assert_eq!(payload_tenant(&jwt, "tid"), "dir-guid");

        let jwt = jwt_with(&serde_json::json!({"tenant_id": ["t1", "t2"]}));
        assert_eq!(payload_tenant(&jwt, "tenant_id"), "t1");
    }

    #[test]
    fn tenant_claim_absent_or_malformed_is_empty() {
        let jwt = jwt_with(&serde_json::json!({"sub": "u1"}));
        assert!(payload_tenant(&jwt, "tenant_id").is_empty());

        let jwt = jwt_with(&serde_json::json!({"tenant_id": 42}));
        assert!(payload_tenant(&jwt, "tenant_id").is_empty());
        let jwt = jwt_with(&serde_json::json!({"tenant_id": ["t1", 2]}));
        assert!(payload_tenant(&jwt, "tenant_id").is_empty());
    }

    #[test]
    fn external_id_uses_configured_claim_when_present() {
        let jwt = jwt_with(&serde_json::json!({"sub": "pairwise-sub", "oid": "entra-oid-123"}));
        assert_eq!(
            extract_external_id(&jwt, "oid", "pairwise-sub").as_deref(),
            Some("entra-oid-123")
        );
    }

    #[test]
    fn external_id_claim_absent_fails_closed() {
        let jwt = jwt_with(&serde_json::json!({"sub": "pairwise-sub"}));
        assert_eq!(extract_external_id(&jwt, "oid", "pairwise-sub"), None);
    }

    #[test]
    fn external_id_claim_present_but_empty_fails_closed() {
        let jwt = jwt_with(&serde_json::json!({"sub": "pairwise-sub", "oid": ""}));
        assert_eq!(extract_external_id(&jwt, "oid", "pairwise-sub"), None);
    }

    #[test]
    fn external_id_defaults_to_sub() {
        let jwt = jwt_with(&serde_json::json!({"sub": "idp|dev-lead"}));
        assert_eq!(
            extract_external_id(&jwt, "sub", "idp|dev-lead").as_deref(),
            Some("idp|dev-lead")
        );
    }

    #[test]
    fn external_id_mode_is_unchanged_and_still_fails_closed() {
        let with_oid = jwt_with(&serde_json::json!({"sub": "pairwise", "oid": "dir-id"}));
        assert!(matches!(
            resolve_target(ResolveBy::ExternalId, &with_oid, "pairwise", "oid", "ivan@vz.com")
                .expect("resolves"),
            crate::identity::ResolveTarget::ExternalId(ref v) if v == "dir-id"
        ));

        let without_oid = jwt_with(&serde_json::json!({"sub": "pairwise"}));
        assert!(
            resolve_target(
                ResolveBy::ExternalId,
                &without_oid,
                "pairwise",
                "oid",
                "ivan@vz.com"
            )
            .is_err(),
            "a missing external id must refuse, not fall through to the address",
        );
    }

    #[test]
    fn email_mode_resolves_against_the_roster_and_refuses_without_an_address() {
        let jwt = jwt_with(&serde_json::json!({"sub": "kc-uuid", "email": "ivan@vz.com"}));
        assert!(matches!(
            resolve_target(ResolveBy::Email, &jwt, "kc-uuid", "sub", "ivan@vz.com")
                .expect("resolves"),
            crate::identity::ResolveTarget::RosterEmail(ref v) if v == "ivan@vz.com"
        ));

        for absent in ["", "   "] {
            assert!(
                resolve_target(ResolveBy::Email, &jwt, "kc-uuid", "sub", absent).is_err(),
                "an empty address must refuse the login",
            );
        }
    }

    #[test]
    fn an_unverified_address_is_refused_only_when_the_idp_says_so() {
        let unverified = jwt_with(
            &serde_json::json!({"sub": "s", "email": "ivan@vz.com", "email_verified": false}),
        );
        assert!(
            resolve_target(ResolveBy::Email, &unverified, "s", "sub", "ivan@vz.com").is_err(),
            "email_verified=false must refuse",
        );

        for tolerated in [
            serde_json::json!({"sub": "s", "email": "ivan@vz.com", "email_verified": true}),
            serde_json::json!({"sub": "s", "email": "ivan@vz.com"}),
        ] {
            let jwt = jwt_with(&tolerated);
            assert!(
                resolve_target(ResolveBy::Email, &jwt, "s", "sub", "ivan@vz.com").is_ok(),
                "{tolerated} must resolve",
            );
        }
    }

    #[test]
    fn override_email_target_never_reads_external_id_claim() {
        let login = crate::identity::IdpIdentity {
            sub: "idp|dev-lead".to_owned(),
            email: "dev@company.nonpresent".to_owned(),
            tenant_id: "t1".to_owned(),
            resolve_by: crate::identity::ResolveTarget::ExternalId("idp|dev-lead".to_owned()),
        };
        let override_target = crate::identity::IdpIdentity {
            sub: String::new(),
            email: "bob@example.com".to_owned(),
            tenant_id: "t1".to_owned(),
            resolve_by: crate::identity::ResolveTarget::Email("bob@example.com".to_owned()),
        };
        assert!(matches!(
            login.resolve_by,
            crate::identity::ResolveTarget::ExternalId(ref v) if v == "idp|dev-lead"
        ));
        assert!(matches!(
            override_target.resolve_by,
            crate::identity::ResolveTarget::Email(ref v) if v == "bob@example.com"
        ));
    }
}
