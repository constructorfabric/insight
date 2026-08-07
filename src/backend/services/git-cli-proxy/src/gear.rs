//! The git-cli-proxy gear: validates config fail-fast, builds the repo store,
//! and mounts the routes on the `api-gateway` host (auth disabled at the host;
//! `/v1` guarded by the service's own bearer token — Airbyte-only consumer).

use std::path::Path;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::api::AppState;
use crate::config::GearConfig;
use crate::engine::disk::Budget;
use crate::engine::store::RepoStore;

/// Config key is the gear name `git-cli-proxy`; env overrides are
/// `APP__gears__git_cli_proxy__config__*`.
#[derive(Default)]
#[toolkit::gear(name = "git-cli-proxy", capabilities = [rest])]
pub struct GitCliProxyGear {
    state: OnceLock<Arc<AppState>>,
}

#[async_trait]
impl Gear for GitCliProxyGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let config: GearConfig = ctx.config()?;
        config.validate()?;
        tracing::info!(config = ?config, "starting git-cli-proxy gear");

        let store = RepoStore::open_cache(
            Path::new(&config.data_dir),
            config.heavy_ops_concurrency,
            Some(config.ca_cert_path.clone()),
            Budget {
                total_bytes: config.disk_budget_bytes,
            },
            config.max_repo_bytes,
        )?;

        let state = AppState {
            store: Arc::new(store),
            config,
        };
        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;
        Ok(())
    }
}

impl RestApiCapability for GitCliProxyGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: axum::Router,
        _openapi: &dyn toolkit::api::OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow::anyhow!("git-cli-proxy gear not initialized"))?
            .clone();
        Ok(crate::api::register_routes(router, state))
    }
}
