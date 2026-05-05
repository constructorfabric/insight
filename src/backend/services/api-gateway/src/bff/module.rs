//! BFF ModKit module.
//!
//! Loads config, builds the OIDC client (discovery + JWKS), wires the
//! Redis-backed session store, and registers `/auth/*` routes.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::Router;
use modkit::api::OpenApiRegistry;
use modkit::context::ModuleCtx;
use modkit::contracts::{Module, RestApiCapability};
use tracing::info;

use crate::bff::config::BffConfig;
use crate::bff::handlers::BffState;
use crate::bff::oidc_client::OidcClient;
use crate::bff::session_store::SessionStore;
use crate::redis_client::RedisShared;

/// BFF module — owns `/auth/*` and the session lifecycle.
#[modkit::module(
    name = "bff",
    deps = ["redis-client"],
    capabilities = [rest]
)]
pub struct BffModule {
    state: OnceLock<Arc<BffState>>,
}

impl Default for BffModule {
    fn default() -> Self {
        Self {
            state: OnceLock::new(),
        }
    }
}

#[async_trait]
impl Module for BffModule {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        let cfg: BffConfig = ctx.config()?;
        cfg.validate()?;

        // Pull the shared Redis client published by the redis-client module.
        let redis = ctx
            .client_hub()
            .get::<RedisShared>()
            .map_err(|e| anyhow::anyhow!("bff: redis-client not available: {e}"))?;

        // Build the OIDC client: discovery + JWKS.
        let redirect_uri = format!("{}/auth/callback", cfg.public_origin.trim_end_matches('/'));
        let oidc = OidcClient::new(
            &cfg.oidc.issuer_url,
            &cfg.oidc.client_id,
            &cfg.oidc.client_secret,
            &redirect_uri,
            cfg.effective_scopes(),
            cfg.oidc.audience.as_deref(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("bff: oidc client init failed: {e}"))?;

        let store = SessionStore::new(redis.clone());

        let state = Arc::new(BffState {
            cfg: Arc::new(cfg),
            oidc: Arc::new(oidc),
            store,
            redis,
        });

        self.state
            .set(state)
            .map_err(|_| anyhow::anyhow!("bff module already initialized"))?;

        info!("bff: initialized");
        Ok(())
    }
}

impl RestApiCapability for BffModule {
    fn register_rest(
        &self,
        _ctx: &ModuleCtx,
        router: Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<Router> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow::anyhow!("bff module not initialized"))?
            .clone();
        Ok(crate::bff::routes::register(router, openapi, state))
    }
}
