use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::api::AppState;
use crate::config::GearConfig;

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

        let db = crate::infra::db::connect(&config.database_url).await?;

        crate::infra::db::assert_schema_compatible(&db).await?;

        let state = AppState { db, config };
        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;
        Ok(())
    }
}

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

pub async fn run_migrate(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    tracing::info!("running migrations");
    let cfg = extract_gear_config(app)?;
    anyhow::ensure!(
        !cfg.database_url.is_empty(),
        "`gears.identity-resolution.config.database_url` is required for migrate"
    );
    crate::infra::db::run_migrations(&cfg.database_url, &cfg).await?;
    Ok(())
}

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

    let metrics = crate::infra::telemetry::MetricsGuard::install(&app.opentelemetry);
    let outcome = seed_and_publish(&cfg, mode, force).await;
    metrics.shutdown();
    outcome
}

async fn seed_and_publish(
    cfg: &crate::config::GearConfig,
    mode: &str,
    force: bool,
) -> Result<(), crate::seed_runner::SeedRunError> {
    let summary = crate::seed_runner::run(cfg, mode, force).await?;
    tracing::info!(?summary, "persons-seed run finished");
    publish_after_seed(crate::sync_runner::run(cfg, false).await)
        .map_err(crate::seed_runner::SeedRunError::Failed)
}

fn publish_after_seed(
    result: Result<crate::domain::sync_service::SyncOutcome, crate::sync_runner::SyncRunError>,
) -> anyhow::Result<()> {
    use crate::sync_runner::SyncRunError;
    match result {
        Ok(outcome) => {
            tracing::info!(?outcome, "persons-sync published the seeded log");
            Ok(())
        }
        Err(SyncRunError::LockBusy) => {
            tracing::warn!("another persons-sync holds the publish lock; skipping");
            Ok(())
        }
        Err(SyncRunError::Guard(msg)) => {
            tracing::warn!(%msg, "persons-sync refused the publish");
            Ok(())
        }
        Err(SyncRunError::Failed(e)) => Err(
            e.context("the seed completed but its publish failed; the resolver snapshot is stale")
        ),
    }
}

pub async fn run_sync(
    app: &toolkit::bootstrap::AppConfig,
    force: bool,
) -> Result<(), crate::sync_runner::SyncRunError> {
    let cfg = extract_gear_config(app).map_err(crate::sync_runner::SyncRunError::Failed)?;
    if cfg.database_url.is_empty() {
        return Err(crate::sync_runner::SyncRunError::Failed(anyhow::anyhow!(
            "`gears.identity-resolution.config.database_url` is required for sync"
        )));
    }
    let summary = crate::sync_runner::run(&cfg, force).await?;
    tracing::info!(?summary, "persons-sync run finished");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sync_service::{SyncOutcome, SyncSummary};
    use crate::sync_runner::SyncRunError;

    fn published() -> SyncOutcome {
        SyncOutcome::Published(SyncSummary {
            rows: 1,
            max_id: Some(1),
            max_created_at: Some("2026-01-01T00:00:00".to_owned()),
            synced_at: "2026-01-01T00:00:01".to_owned(),
        })
    }

    #[test]
    fn a_published_seed_is_a_success() {
        assert!(publish_after_seed(Ok(published())).is_ok());
    }

    #[test]
    fn an_already_current_snapshot_is_a_success() {
        let outcome = SyncOutcome::AlreadyCurrent { max_id: Some(1) };
        assert!(publish_after_seed(Ok(outcome)).is_ok());
    }

    #[test]
    fn a_busy_publish_lock_does_not_fail_the_seed() {
        assert!(publish_after_seed(Err(SyncRunError::LockBusy)).is_ok());
    }

    #[test]
    fn a_guard_refusal_does_not_fail_the_seed() {
        let refusal = SyncRunError::Guard("persons log is empty".to_owned());
        assert!(publish_after_seed(Err(refusal)).is_ok());
    }

    #[test]
    fn a_failed_publish_fails_the_seed_run() {
        let failed = SyncRunError::Failed(anyhow::anyhow!("clickhouse unreachable"));
        let Err(err) = publish_after_seed(Err(failed)) else {
            panic!("a stale snapshot must surface as a failed run");
        };
        assert!(format!("{err:#}").contains("publish failed"), "{err:#}");
    }

    #[test]
    fn a_failure_mentioning_the_guard_stays_a_failure() {
        let failed = SyncRunError::Failed(anyhow::anyhow!("persons log is empty"));
        assert!(publish_after_seed(Err(failed)).is_err());
    }
}
