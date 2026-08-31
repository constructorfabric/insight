//! The previews gear.
//!
//! Runs on the `api-gateway` system gear (the REST host) under
//! `toolkit::bootstrap::run_server`. [`PreviewsGear::init`] builds the
//! runtime (the in-cluster Kubernetes client) and spawns the TTL sweep;
//! [`register_rest`] mounts the experiment routes on the host router.
//!
//! [`register_rest`]: PreviewsGear::register_rest

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::api::AppState;
use crate::config::GearConfig;
use crate::infra::cluster::Cluster;
use crate::infra::registry::Registry;

/// Previews gear. Capability: `rest` (HTTP surface). Config key is the gear
/// name `previews`; env overrides are `APP__gears__previews__config__*`.
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

        // Plain interval sweep, not a leader-elected job: the service runs a
        // single replica, and a duplicated pass would only race deletes of
        // already-expired objects (404s are tolerated).
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
