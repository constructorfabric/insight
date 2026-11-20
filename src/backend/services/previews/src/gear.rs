use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::api::AppState;
use crate::config::GearConfig;
use crate::infra::cluster::Cluster;
use crate::infra::registry::Registry;

#[derive(Default)]
#[toolkit::gear(name = "previews", capabilities = [rest])]
pub struct PreviewsGear {
    state: OnceLock<Arc<AppState>>,
}

#[async_trait]
impl Gear for PreviewsGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let config: GearConfig = ctx.config()?;
        config.validate()?;
        tracing::info!(namespace = config.namespace, "starting previews gear");

        let cluster = Cluster::connect(config.namespace.clone()).await?;

        tokio::spawn(crate::sweep::run(
            cluster.clone(),
            config.sweep_interval_secs,
        ));

        let registry = if config.registry_url.is_empty() {
            None
        } else {
            Some(Registry::connect(
                &config.registry_url,
                &config.registry_token,
            )?)
        };

        let state = AppState {
            cluster,
            registry,
            config,
            create_gate: tokio::sync::Mutex::new(()),
        };
        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;
        Ok(())
    }
}

impl RestApiCapability for PreviewsGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: axum::Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow::anyhow!("previews gear not initialized"))?
            .clone();
        Ok(crate::api::register_routes(router, openapi, state))
    }
}
