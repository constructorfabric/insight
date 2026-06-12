//! BFF module configuration.
//!
//! All knobs live under `modules.bff.config` in the api-gateway YAML.
//! Defaults match `cpt-insightspec-fr-bff-*` requirements unless explicitly
//! noted; production values should be set via Helm.

use serde::Deserialize;

/// BFF module configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BffConfig {
    /// OIDC settings.
    pub oidc: OidcConfig,
    /// Session lifecycle settings.
    pub session: SessionConfig,
    /// CSRF settings.
    pub csrf: CsrfConfig,
    /// Public origin where the BFF serves the SPA's redirect target.
    /// Used to build absolute callback URLs and the post-login redirect.
    /// Required.
    pub public_origin: String,
    /// Default tenant assigned to every authenticated user until the
    /// Identity Service mapping lands. Single-tenant deployments only.
    pub default_tenant_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OidcConfig {
    /// OIDC issuer URL — `<issuer>/.well-known/openid-configuration` is
    /// fetched at module init.
    pub issuer_url: String,
    /// Confidential client ID.
    pub client_id: String,
    /// Confidential client secret.
    pub client_secret: String,
    /// Scopes to request (`openid` is appended automatically if missing).
    pub scopes: Vec<String>,
    /// Optional override for the OIDC `audience` claim validation. Defaults
    /// to `client_id`.
    pub audience: Option<String>,
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            scopes: vec![
                "openid".to_owned(),
                "profile".to_owned(),
                "email".to_owned(),
            ],
            audience: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    /// Session cookie TTL. Spec range: 30 s – 1 h. Default 120 s.
    pub ttl_seconds: u32,
    /// Hard cap on session lifetime across refreshes. Spec range:
    /// 1 h – 24 h. Default 8 h.
    pub absolute_lifetime_seconds: u32,
    /// `refresh_at = expires_at - safety_margin + jitter`. Default 30 s.
    pub refresh_safety_margin_seconds: u16,
    /// Total jitter window applied to `refresh_at` (uniform in `±half/2`).
    /// Default 10 s.
    pub refresh_jitter_seconds: u16,
    /// Grace TTL for `bff:swap:{old_sid}` after a refresh-rotation. Default
    /// 250 ms.
    pub refresh_grace_ms: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ttl_seconds: 120,
            absolute_lifetime_seconds: 8 * 3600,
            refresh_safety_margin_seconds: 30,
            refresh_jitter_seconds: 10,
            refresh_grace_ms: 250,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CsrfConfig {
    /// Allowlist of `Origin` values accepted as the secondary CSRF defense.
    /// Empty means fail-closed: every state-changing `/auth/*` request must
    /// carry a valid `X-CSRF-Token`.
    pub origins: Vec<String>,
}

impl BffConfig {
    /// Validate the loaded config against spec ranges. Called once at
    /// module init.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.public_origin.is_empty() {
            anyhow::bail!("bff: public_origin is required");
        }
        let parsed = url::Url::parse(&self.public_origin)
            .map_err(|e| anyhow::anyhow!("bff: public_origin is not a valid URL: {e}"))?;
        validate_public_origin(&parsed)?;

        if self.oidc.issuer_url.is_empty() {
            anyhow::bail!("bff: oidc.issuer_url is required");
        }
        if self.oidc.client_id.is_empty() {
            anyhow::bail!("bff: oidc.client_id is required");
        }
        if self.oidc.client_secret.is_empty() {
            anyhow::bail!("bff: oidc.client_secret is required");
        }
        if self.default_tenant_id.trim().is_empty() {
            anyhow::bail!("bff: default_tenant_id is required");
        }

        // cpt-insightspec-nfr-bff-session-ttl
        if !(30..=3600).contains(&self.session.ttl_seconds) {
            anyhow::bail!(
                "bff: session.ttl_seconds must be in [30, 3600], got {}",
                self.session.ttl_seconds
            );
        }
        if !(3600..=86_400).contains(&self.session.absolute_lifetime_seconds) {
            anyhow::bail!(
                "bff: session.absolute_lifetime_seconds must be in [3600, 86400], got {}",
                self.session.absolute_lifetime_seconds
            );
        }
        if u32::from(self.session.refresh_safety_margin_seconds) >= self.session.ttl_seconds {
            anyhow::bail!(
                "bff: session.refresh_safety_margin_seconds ({}) must be < session.ttl_seconds ({})",
                self.session.refresh_safety_margin_seconds,
                self.session.ttl_seconds
            );
        }
        if u32::from(self.session.refresh_jitter_seconds) >= self.session.ttl_seconds {
            anyhow::bail!(
                "bff: session.refresh_jitter_seconds ({}) must be < session.ttl_seconds ({})",
                self.session.refresh_jitter_seconds,
                self.session.ttl_seconds
            );
        }
        if self.session.refresh_grace_ms == 0 || self.session.refresh_grace_ms > 5_000 {
            anyhow::bail!(
                "bff: session.refresh_grace_ms must be in (0, 5000], got {}",
                self.session.refresh_grace_ms
            );
        }

        Ok(())
    }

    /// Final list of OIDC scopes with `openid` guaranteed present.
    #[must_use]
    pub fn effective_scopes(&self) -> Vec<String> {
        let mut scopes = self.oidc.scopes.clone();
        if !scopes.iter().any(|s| s == "openid") {
            scopes.insert(0, "openid".to_owned());
        }
        scopes
    }
}

/// Defense-in-depth validation for `public_origin`: HTTPS only, except for
/// local-dev hosts (`localhost`, `127.0.0.1`). Reject query/fragment/userinfo
/// and any path beyond `/` since the value is concatenated with `"/auth/..."`
/// at the call site. Runtime traffic is still enforced HTTPS-only at the
/// ingress (NFR `cpt-insightspec-nfr-bff-https-only`); this guards the config.
fn validate_public_origin(u: &url::Url) -> anyhow::Result<()> {
    let host = u.host_str().unwrap_or("");
    let is_local = host == "localhost" || host == "127.0.0.1";
    match u.scheme() {
        "https" => {}
        "http" if is_local => {}
        s => anyhow::bail!(
            "bff: public_origin must use https (got `{s}://{host}`); http allowed only for localhost"
        ),
    }
    if !u.username().is_empty() || u.password().is_some() {
        anyhow::bail!("bff: public_origin must not include userinfo");
    }
    if u.query().is_some() {
        anyhow::bail!("bff: public_origin must not include a query string");
    }
    if u.fragment().is_some() {
        anyhow::bail!("bff: public_origin must not include a fragment");
    }
    let path = u.path();
    if !(path.is_empty() || path == "/") {
        anyhow::bail!("bff: public_origin must not include a path (got `{path}`)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_cfg() -> BffConfig {
        BffConfig {
            public_origin: "https://insight.example.com".to_owned(),
            default_tenant_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            oidc: OidcConfig {
                issuer_url: "https://idp.example.com".to_owned(),
                client_id: "insight-bff".to_owned(),
                client_secret: "secret".to_owned(),
                ..OidcConfig::default()
            },
            ..BffConfig::default()
        }
    }

    #[test]
    fn validate_accepts_good_config() {
        good_cfg().validate().expect("should pass");
    }

    #[test]
    fn validate_rejects_short_session_ttl() {
        let mut c = good_cfg();
        c.session.ttl_seconds = 10;
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_long_session_ttl() {
        let mut c = good_cfg();
        c.session.ttl_seconds = 7200;
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_short_absolute_lifetime() {
        let mut c = good_cfg();
        c.session.absolute_lifetime_seconds = 300;
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_safety_margin_geq_ttl() {
        let mut c = good_cfg();
        c.session.refresh_safety_margin_seconds = 120;
        assert!(c.validate().is_err());
    }

    #[test]
    fn effective_scopes_inserts_openid_when_missing() {
        let mut c = good_cfg();
        c.oidc.scopes = vec!["profile".to_owned()];
        assert_eq!(
            c.effective_scopes(),
            vec!["openid".to_owned(), "profile".to_owned()]
        );
    }

    #[test]
    fn effective_scopes_keeps_openid_when_present() {
        let c = good_cfg();
        let s = c.effective_scopes();
        assert_eq!(s.iter().filter(|x| x.as_str() == "openid").count(), 1);
    }

    #[test]
    fn validate_rejects_empty_public_origin() {
        let mut c = good_cfg();
        c.public_origin = String::new();
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_issuer() {
        let mut c = good_cfg();
        c.oidc.issuer_url = String::new();
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_default_tenant_id() {
        let mut c = good_cfg();
        c.default_tenant_id = String::new();
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_blank_default_tenant_id() {
        let mut c = good_cfg();
        c.default_tenant_id = "   ".to_owned();
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_accepts_localhost_http_public_origin() {
        let mut c = good_cfg();
        c.public_origin = "http://localhost:8080".to_owned();
        c.validate().expect("localhost http should pass");
    }

    #[test]
    fn validate_accepts_loopback_http_public_origin() {
        let mut c = good_cfg();
        c.public_origin = "http://127.0.0.1:8080".to_owned();
        c.validate().expect("127.0.0.1 http should pass");
    }

    #[test]
    fn validate_rejects_non_localhost_http_public_origin() {
        let mut c = good_cfg();
        c.public_origin = "http://insight.example.com".to_owned();
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_https_with_userinfo_public_origin() {
        let mut c = good_cfg();
        c.public_origin = "https://user:pass@insight.example.com".to_owned();
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_public_origin_with_query() {
        let mut c = good_cfg();
        c.public_origin = "https://insight.example.com/?x=1".to_owned();
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_public_origin_with_fragment() {
        let mut c = good_cfg();
        c.public_origin = "https://insight.example.com/#f".to_owned();
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_public_origin_with_path() {
        let mut c = good_cfg();
        c.public_origin = "https://insight.example.com/app".to_owned();
        assert!(c.validate().is_err());
    }
}
