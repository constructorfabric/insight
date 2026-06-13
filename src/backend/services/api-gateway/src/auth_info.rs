//! Auth info module — public endpoint that serves OIDC configuration to the frontend.
//!
//! `GET /v1/auth/config` — no authentication required.
//!
//! Returns the OIDC provider details the frontend needs to initiate the
//! Authorization Code flow with PKCE (redirect to login page, token exchange).

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use axum::{Json, Router};
use modkit::api::{OpenApiRegistry, OperationBuilder};
use modkit::context::ModuleCtx;
use modkit::contracts::{Module, RestApiCapability};
use serde::{Deserialize, Serialize};

/// OIDC configuration served to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthInfoResponse {
    /// OIDC issuer URL (e.g., `https://dev-12345.okta.com/oauth2/default`).
    pub issuer_url: String,
    /// OIDC client ID for the frontend application.
    pub client_id: String,
    /// Redirect URI after login (frontend callback URL).
    pub redirect_uri: String,
    /// Scopes to request from the OIDC provider.
    pub scopes: Vec<String>,
    /// OIDC response type (always "code" for Authorization Code flow).
    pub response_type: String,
}

impl AuthInfoResponse {
    /// Build the frontend response from module config: split the space-separated
    /// `scopes` (OAuth2 wire format) into a list, and pin `response_type` to the
    /// Authorization-Code-flow value. Extracted from `register_rest` so the
    /// transformation is unit-testable without the module runtime.
    pub(crate) fn from_config(config: &AuthInfoConfig) -> Self {
        Self {
            issuer_url: config.issuer_url.clone(),
            client_id: config.client_id.clone(),
            redirect_uri: config.redirect_uri.clone(),
            scopes: config
                .scopes
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            response_type: "code".to_owned(),
        }
    }
}

/// Module configuration (from YAML).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthInfoConfig {
    /// OIDC issuer URL. Should match the OIDC plugin's `issuer_url`.
    pub issuer_url: String,
    /// OIDC client ID for the frontend (public client, no secret).
    pub client_id: String,
    /// Frontend callback URL after OIDC login.
    pub redirect_uri: String,
    /// Scopes to request, as a space-separated string (matches OAuth2's wire
    /// format). Stored as `String` so the standard
    /// `APP__modules__auth-info__config__scopes` env-var override works
    /// without a custom Vec deserializer; split on whitespace when building
    /// the response. IdP-specific:
    ///   Entra v2 single-app: "openid profile email api://<clientId>/Access.Default"
    ///   Okta:                "openid profile email <api-name>.<scope>"
    pub scopes: String,
}

/// Auth info module — serves OIDC config to the frontend.
#[modkit::module(
    name = "auth-info",
    capabilities = [rest]
)]
pub struct AuthInfoModule {
    config: OnceLock<Arc<AuthInfoConfig>>,
}

impl Default for AuthInfoModule {
    fn default() -> Self {
        Self {
            config: OnceLock::new(),
        }
    }
}

#[async_trait]
impl Module for AuthInfoModule {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        let config: AuthInfoConfig = ctx.config()?;

        if config.issuer_url.is_empty() {
            tracing::warn!(
                "auth-info: issuer_url is empty. \
                 /auth/config endpoint will return empty OIDC config. \
                 Set modules.auth-info.config.issuer_url."
            );
        }
        if config.scopes.split_whitespace().next().is_none() {
            tracing::warn!(
                "auth-info: scopes is empty. SPA will request no OIDC scopes \
                 and IdPs will fall back to default audiences (Entra → Microsoft Graph), \
                 producing access tokens the gateway can't validate. \
                 Set modules.auth-info.config.scopes (space-separated)."
            );
        }

        self.config
            .set(Arc::new(config))
            .map_err(|_| anyhow::anyhow!("auth-info module already initialized"))?;

        Ok(())
    }
}

impl RestApiCapability for AuthInfoModule {
    fn register_rest(
        &self,
        _ctx: &ModuleCtx,
        router: Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<Router> {
        let config = self
            .config
            .get()
            .ok_or_else(|| anyhow::anyhow!("auth-info not initialized"))?
            .clone();

        let response = AuthInfoResponse::from_config(&config);

        let handler = move || {
            let resp = response.clone();
            async move { Json(resp) }
        };

        let router = OperationBuilder::new(Method::GET, "/v1/auth/config")
            .summary("OIDC configuration for frontend")
            .description("Returns OIDC provider details for the Authorization Code flow with PKCE. No authentication required.")
            .public()
            .json_response(StatusCode::OK, "OIDC configuration")
            .standard_errors(openapi)
            .handler(handler)
            .register(router, openapi);

        tracing::info!("registered public endpoint: GET /v1/auth/config");
        Ok(router)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type R = Result<(), Box<dyn std::error::Error>>;

    fn cfg(scopes: &str) -> AuthInfoConfig {
        AuthInfoConfig {
            issuer_url: "https://idp.example/oauth2".to_owned(),
            client_id: "spa-client".to_owned(),
            redirect_uri: "https://app.example/callback".to_owned(),
            scopes: scopes.to_owned(),
        }
    }

    #[test]
    fn from_config_splits_space_separated_scopes() {
        let r = AuthInfoResponse::from_config(&cfg("openid profile email"));
        assert_eq!(r.scopes, vec!["openid", "profile", "email"]);
    }

    #[test]
    fn from_config_collapses_extra_whitespace_and_tabs() {
        // OAuth2 wire format is space-separated; be liberal about spacing.
        let r = AuthInfoResponse::from_config(&cfg("  openid\tprofile   email \n"));
        assert_eq!(r.scopes, vec!["openid", "profile", "email"]);
    }

    #[test]
    fn from_config_empty_scopes_yields_empty_vec() {
        assert!(AuthInfoResponse::from_config(&cfg("")).scopes.is_empty());
        assert!(AuthInfoResponse::from_config(&cfg("   ")).scopes.is_empty());
    }

    #[test]
    fn from_config_response_type_is_always_code() {
        assert_eq!(
            AuthInfoResponse::from_config(&cfg("openid")).response_type,
            "code"
        );
    }

    #[test]
    fn from_config_passes_through_identity_fields() {
        let r = AuthInfoResponse::from_config(&cfg("openid"));
        assert_eq!(r.issuer_url, "https://idp.example/oauth2");
        assert_eq!(r.client_id, "spa-client");
        assert_eq!(r.redirect_uri, "https://app.example/callback");
    }

    #[test]
    fn config_default_is_all_empty() {
        let c = AuthInfoConfig::default();
        assert!(c.issuer_url.is_empty());
        assert!(c.client_id.is_empty());
        assert!(c.redirect_uri.is_empty());
        assert!(c.scopes.is_empty());
    }

    #[test]
    fn config_parses_known_fields() -> R {
        let c: AuthInfoConfig = serde_json::from_str(
            r#"{"issuer_url":"https://i","client_id":"c","redirect_uri":"r","scopes":"openid email"}"#,
        )?;
        assert_eq!(c.client_id, "c");
        assert_eq!(c.scopes, "openid email");
        Ok(())
    }

    #[test]
    fn config_rejects_unknown_fields() {
        // deny_unknown_fields guards against typo'd / stale config keys.
        let err = serde_json::from_str::<AuthInfoConfig>(r#"{"issuer":"oops"}"#);
        assert!(err.is_err(), "unknown field must be rejected");
    }

    #[test]
    fn response_json_roundtrips() -> R {
        let r = AuthInfoResponse::from_config(&cfg("openid profile"));
        let json = serde_json::to_string(&r)?;
        let back: AuthInfoResponse = serde_json::from_str(&json)?;
        assert_eq!(r, back);
        // wire shape the frontend depends on
        assert!(json.contains("\"response_type\":\"code\""));
        assert!(json.contains("\"scopes\":[\"openid\",\"profile\"]"));
        Ok(())
    }

    #[test]
    fn module_default_is_uninitialized() {
        let m = AuthInfoModule::default();
        assert!(m.config.get().is_none(), "config unset before init");
    }
}
