//! The identity-resolution gear.
//!
//! Runs on the `api-gateway` system gear (the REST host) under
//! `toolkit::bootstrap::run_server`. [`IdentityResolutionGear::init`] builds the
//! runtime (MariaDB pool + persons-seed worker); [`register_rest`] mounts the
//! profile-read and persons-seed routes on the host router.
//!
//! [`register_rest`]: IdentityResolutionGear::register_rest

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::api::AppState;
use crate::config::GearConfig;

/// Identity-resolution gear. Capability: `rest` (HTTP surface). Config key is
/// the gear name `identity-resolution`; env overrides are
/// `APP__gears__identity-resolution__config__*`.
#[derive(Default)]
#[toolkit::gear(name = "identity-resolution", capabilities = [rest])]
pub struct IdentityResolutionGear {
    state: OnceLock<Arc<AppState>>,
}

#[async_trait]
impl Gear for IdentityResolutionGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let config: GearConfig = ctx.config()?;
        tracing::info!("starting identity-resolution gear");

        // Self-managed MariaDB pool (same approach as the analytics gear).
        let db = crate::infra::db::connect(&config.database_url).await?;

        // Persons-seed background worker: drains a job queue and runs each seed.
        // A single spawned task (like the analytics validators) owns the queue.
        // Capacity matches the .NET `PersonsSeedQueue` bound (100).
        let (seed_tx, seed_rx) = tokio::sync::mpsc::channel(100);
        let worker_db = db.clone();
        let worker_config = config.clone();
        tokio::spawn(async move {
            crate::api::seed::run_worker(seed_rx, worker_db, worker_config).await;
        });

        let state = AppState {
            db,
            config,
            seed_tx,
        };
        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;
        Ok(())
    }
}

impl RestApiCapability for IdentityResolutionGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: axum::Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow::anyhow!("identity-resolution gear not initialized"))?
            .clone();
        Ok(crate::api::register_routes(router, openapi, state))
    }
}
