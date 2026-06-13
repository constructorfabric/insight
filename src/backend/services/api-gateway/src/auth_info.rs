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
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder};
use toolkit::context::GearCtx;
use toolkit::contracts::{Gear, RestApiCapability};

/// OIDC configuration served to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Gear configuration (from YAML).
#[derive(Debug, Clone, Default, Deserialize)]
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
    /// `APP__gears__auth-info__config__scopes` env-var override works
    /// without a custom Vec deserializer; split on whitespace when building
    /// the response. IdP-specific:
    ///   Entra v2 single-app: "openid profile email api://<clientId>/Access.Default"
    ///   Okta:                "openid profile email <api-name>.<scope>"
    pub scopes: String,
}

/// Auth info module — serves OIDC config to the frontend.
#[toolkit::gear(
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
impl Gear for AuthInfoModule {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let config: AuthInfoConfig = ctx.config()?;

        if config.issuer_url.is_empty() {
            tracing::warn!(
                "auth-info: issuer_url is empty. \
                 /auth/config endpoint will return empty OIDC config. \
                 Set gears.auth-info.config.issuer_url."
            );
        }
        if config.scopes.split_whitespace().next().is_none() {
            tracing::warn!(
                "auth-info: scopes is empty. SPA will request no OIDC scopes \
                 and IdPs will fall back to default audiences (Entra → Microsoft Graph), \
                 producing access tokens the gateway can't validate. \
                 Set gears.auth-info.config.scopes (space-separated)."
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
        _ctx: &GearCtx,
        router: Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<Router> {
        let config = self
            .config
            .get()
            .ok_or_else(|| anyhow::anyhow!("auth-info not initialized"))?
            .clone();

        let response = AuthInfoResponse {
            issuer_url: config.issuer_url.clone(),
            client_id: config.client_id.clone(),
            redirect_uri: config.redirect_uri.clone(),
            scopes: config
                .scopes
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            response_type: "code".to_owned(),
        };

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
