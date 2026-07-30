//! The identity-resolution gear.
//!
//! Runs on the `api-gateway` system gear (the REST host) under
//! `toolkit::bootstrap::run_server`. [`IdentityResolutionGear::init`] builds the
//! runtime (MariaDB pool); [`register_rest`] mounts the profile-read and
//! persons-seed-journal routes on the host router. The seed itself runs via
//! the `seed` CLI subcommand ([`run_seed`]), not in this process.
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
        // No background workers: the persons-seed runs as the `seed` CLI
        // subcommand (CronJob / manual Job — see `crate::seed_runner`), so the
        // server process only serves reads + the operations journal.
        let db = crate::infra::db::connect(&config.database_url).await?;

        let state = AppState { db, config };
        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;
        Ok(())
    }
}

/// Pull `gears.identity-resolution.config` out of the loaded `AppConfig` and
/// deserialize it into [`GearConfig`] — for subcommands that run outside the
/// gear lifecycle (same helper shape as the analytics service).
fn extract_gear_config(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<GearConfig> {
    let raw = app
        .gears
        .get("identity-resolution")
        .and_then(|v| v.get("config"))
        .ok_or_else(|| {
            anyhow::anyhow!("missing `gears.identity-resolution.config` section in configuration")
        })?;
    let cfg: GearConfig = serde_json::from_value(raw.clone())?;
    Ok(cfg)
}

/// `migrate` subcommand: apply pending schema migrations + the first-admin
/// bootstrap, then exit. Runs on a dedicated single-connection session (the
/// advisory lock in `run_migrations` is session-scoped).
///
/// # Errors
///
/// Returns an error when the config section or `database_url` is missing, the
/// migration lock cannot be acquired, or a migration/bootstrap step fails.
pub async fn run_migrate(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    tracing::info!("running migrations");
    let cfg = extract_gear_config(app)?;
    anyhow::ensure!(
        !cfg.database_url.is_empty(),
        "`gears.identity-resolution.config.database_url` is required for migrate"
    );
    let db = crate::infra::db::connect_single(&cfg.database_url).await?;
    // Migrations AND the first-admin bootstrap run under one advisory lock —
    // see `run_migrations` on why the bootstrap must stay inside the critical
    // section (unconstrained INSERT … WHERE NOT EXISTS between replicas).
    crate::infra::db::run_migrations(&db, &cfg).await?;
    Ok(())
}

/// `seed` subcommand: run one persons-seed via [`crate::seed_runner`] and
/// exit. Same out-of-lifecycle config extraction as `migrate`.
///
/// # Errors
///
/// [`crate::seed_runner::SeedRunError`] — the caller maps each variant to a
/// distinct process exit code.
pub async fn run_seed(
    app: &toolkit::bootstrap::AppConfig,
    mode: &str,
    force: bool,
) -> Result<(), crate::seed_runner::SeedRunError> {
    let cfg = extract_gear_config(app).map_err(crate::seed_runner::SeedRunError::Failed)?;
    if cfg.database_url.is_empty() {
        return Err(crate::seed_runner::SeedRunError::Failed(anyhow::anyhow!(
            "`gears.identity-resolution.config.database_url` is required for seed"
        )));
    }
    let summary = crate::seed_runner::run(&cfg, mode, force).await?;
    tracing::info!(?summary, "persons-seed run finished");
    Ok(())
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
