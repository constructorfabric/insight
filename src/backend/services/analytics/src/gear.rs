//! The `analytics` gear.
//!
//! Hosts the analytics REST surface on the `api-gateway` system gear (the REST
//! host) under `toolkit::bootstrap::run_server`. All runtime construction that
//! used to live in `main.rs::run_server` — the self-managed MariaDB pool, its
//! migrations + startup probe, the ClickHouse / Identity clients and the
//! metric-definition validator — happens in [`AnalyticsApiGear::init`]. Auth is
//! disabled on this host; the tenant override layer lives in [`crate::auth`].
//!
//! The DB is self-managed (LOCKED DECISION): we do NOT use the toolkit `db`
//! capability — ClickHouse is not a toolkit-db backend, and the gear keeps its
//! own sea-orm pool in `AppState`.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::config::GearConfig;
use crate::domain::external_links::ExternalSourceRegistry;
use crate::{api, infra};

/// Analytics API gear. Capabilities: `rest` only (the background validator and
/// contract-version passes are `tokio::spawn`ed in `init`; no
/// `stateful`/`RunnableCapability`).
// Config key is the gear name `analytics`; env overrides are
// `APP__gears__analytics__config__*`.
#[toolkit::gear(
    name = "analytics",
    capabilities = [rest]
)]
pub struct AnalyticsApiGear {
    state: OnceLock<Arc<api::AppState>>,
}

impl Default for AnalyticsApiGear {
    fn default() -> Self {
        Self {
            state: OnceLock::new(),
        }
    }
}

#[async_trait]
impl Gear for AnalyticsApiGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let cfg: GearConfig = ctx.config()?;
        let external_links = ExternalSourceRegistry::new(&cfg.external_sources)?;
        tracing::info!("starting analytics gear");

        // Connect to MariaDB (self-managed pool — LOCKED DECISION).
        let db = infra::db::connect(&cfg.database_url).await?;

        // INVARIANT: server boot stays read-only — migrations and the
        // builtin-definition converge belong to the `migrate` entrypoint, so
        // N replicas cannot race and an older image can be redeployed over a
        // newer forward-only schema.
        infra::db::assert_schema_compatible(&db).await?;

        // Refuse to start if any required CHECK constraint is missing. See
        // `infra/db/check_probe` and DESIGN §2.2
        // `cpt-metric-cat-constraint-mariadb-check`.
        infra::db::check_probe::assert_required_checks(&db).await?;

        // Connect to ClickHouse. Observation views execute live and their
        // union branches run as parallel pipelines, so per-query memory
        // scales with thread count: four threads measured at the same
        // latency as unbounded parallelism with ~40% less peak memory on
        // the widest observation view. The memory ceiling makes a
        // pathological query fail alone with a typed error instead of
        // pushing the shared server tracker over its limit and killing
        // every in-flight query on the instance.
        let mut ch_config =
            insight_clickhouse::Config::new(&cfg.clickhouse_url, &cfg.clickhouse_database)
                .with_query_max_threads(4)
                .with_query_max_memory_bytes(1_610_612_736);
        if let (Some(user), Some(password)) = (&cfg.clickhouse_user, &cfg.clickhouse_password) {
            ch_config = ch_config.with_auth(user, password);
        }
        let ch = insight_clickhouse::Client::new(ch_config);

        // Identity client.
        let identity = infra::identity::IdentityClient::new(&cfg.identity_url)?;

        let metric_definition_validator =
            crate::domain::metric_definitions::MetricDefinitionValidator::new(
                db.clone(),
                ch.clone(),
            );

        let contract_ch = ch.clone();

        let anthropic = infra::anthropic::AnthropicClient::new(
            &cfg.ai_assist.api_base,
            std::time::Duration::from_secs(cfg.ai_assist.request_timeout_secs),
        )?;
        let ai_calls = Arc::new(tokio::sync::Semaphore::new(cfg.ai_assist.max_concurrent));

        let state = api::AppState {
            db,
            ch,
            identity,
            anthropic,
            ai_calls,
            config: cfg,
            external_links,
        };

        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;

        // INVARIANT: periodic and never gating boot — the stamp lands after
        // boot (post-install migrate hook) and a later in-place bump must
        // surface without a pod restart.
        tokio::spawn(async move {
            crate::domain::contract_version::run(&contract_ch).await;
        });
        // Periodic, not one-shot: the managed observation views are
        // dbt-created after boot on a fresh deploy, and the registry has no
        // write path that would re-trigger probing.
        tokio::spawn(metric_definition_validator.run());

        Ok(())
    }
}

impl RestApiCapability for AnalyticsApiGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: axum::Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow::anyhow!("analytics gear not initialized"))?
            .clone();

        Ok(api::register_routes(router, openapi, state))
    }
}

/// `analytics migrate`: run migrations + the builtin-definition converge + the
/// startup probe, then exit.
///
/// This is the only path that writes schema or builtin registry rows. Both run
/// inside one advisory-locked, single-connection session so that concurrent
/// migrators cannot double-apply non-transactional DDL, and so the converge's
/// read-then-write sequences cannot interleave.
///
/// # Errors
///
/// Returns an error if config extraction, DB connect, the migration lock,
/// migrations, the converge, or the probe fails.
pub async fn run_migrate(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    tracing::info!("running migrations");

    let cfg = extract_gear_config(app)?;

    insight_migration::with_migration_session(
        &cfg.database_url,
        infra::db::MIGRATION_LOCK,
        infra::db::MIGRATION_LOCK_TIMEOUT,
        |db| async move {
            use sea_orm_migration::MigratorTrait;
            crate::migration::Migrator::up(&db, None).await?;
            tracing::info!("migrations applied");

            // The converge owns builtin metric definitions: it upserts the code
            // registry and disables rows the registry no longer carries. It
            // must not run on a server boot — an older image would disable the
            // definitions a newer release introduced.
            crate::domain::metric_definitions::reconcile_builtin_definitions(&db).await?;

            // Same probe as `init`. An operator running `migrate` standalone
            // wants the integrity signal too (DESIGN §2.2).
            infra::db::check_probe::assert_required_checks(&db).await?;
            Ok(())
        },
    )
    .await?;

    tracing::info!("migrations complete");
    Ok(())
}

/// Pull `gears.analytics.config` out of the loaded `AppConfig` and
/// deserialize it into [`GearConfig`].
fn extract_gear_config(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<GearConfig> {
    let raw = app
        .gears
        .get("analytics")
        .and_then(|v| v.get("config"))
        .ok_or_else(|| {
            anyhow::anyhow!("missing `gears.analytics.config` section in configuration")
        })?;
    let cfg: GearConfig = serde_json::from_value(raw.clone())?;
    Ok(cfg)
}

/// Validate the analytics gear config without touching the database — used by
/// the `check` subcommand. Proves `gears.analytics.config` is present,
/// deserializes, and carries the connection strings the gear needs at boot.
///
/// # Errors
///
/// Returns an error if the section is missing/undeserializable or a required
/// URL is empty.
pub fn check_config(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    let cfg = extract_gear_config(app)?;
    if cfg.database_url.trim().is_empty() {
        anyhow::bail!(
            "gears.analytics.config.database_url is empty (set \
             APP__gears__analytics__config__database_url)"
        );
    }
    if cfg.clickhouse_url.trim().is_empty() {
        anyhow::bail!(
            "gears.analytics.config.clickhouse_url is empty (set \
             APP__gears__analytics__config__clickhouse_url)"
        );
    }
    ExternalSourceRegistry::new(&cfg.external_sources)?;
    if cfg.ai_assist.enabled {
        if cfg.ai_assist.max_concurrent == 0 {
            anyhow::bail!(
                "gears.analytics.config.ai_assist.max_concurrent is 0, so no explain call \
                 could ever run (set \
                 APP__gears__analytics__config__ai_assist__max_concurrent to at least 1)"
            );
        }
        if cfg.ai_assist.request_timeout_secs == 0 {
            anyhow::bail!(
                "gears.analytics.config.ai_assist.request_timeout_secs is 0, so every model \
                 call would abort before it starts (set \
                 APP__gears__analytics__config__ai_assist__request_timeout_secs to at least 1)"
            );
        }
        cfg.ai_assist.encryption_key().map_err(|e| {
            anyhow::anyhow!(
                "gears.analytics.config.ai_assist is enabled but its \
                 token_encryption_key is unusable: {e} (set \
                 APP__gears__analytics__config__ai_assist__token_encryption_key \
                 to base64 of 32 random bytes)"
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use toolkit::bootstrap::AppConfig;

    fn cfg(config: serde_json::Value) -> AppConfig {
        let mut c = AppConfig::default();
        let mut section = serde_json::Map::new();
        section.insert("config".to_owned(), config);
        c.gears
            .insert("analytics".to_owned(), serde_json::Value::Object(section));
        c
    }

    #[test]
    fn extract_gear_config_missing_section_errors() {
        assert!(extract_gear_config(&AppConfig::default()).is_err());
    }

    #[test]
    fn check_config_ok_with_required_urls() {
        let c = cfg(json!({
            "database_url": "mysql://h:3306/db",
            "clickhouse_url": "http://h:8123",
        }));
        assert!(check_config(&c).is_ok());
    }

    #[test]
    fn check_config_errs_on_invalid_external_source_url() {
        let c = cfg(json!({
            "database_url": "mysql://h:3306/db",
            "clickhouse_url": "http://h:8123",
            "external_sources": [{
                "id": "source-a",
                "provider": "github",
                "web_base_url": "https://code.example.test#fragment"
            }]
        }));

        assert!(check_config(&c).is_err());
    }

    #[test]
    fn check_config_errs_when_ai_assist_is_on_without_a_usable_key() {
        let c = cfg(json!({
            "database_url": "mysql://h:3306/db",
            "clickhouse_url": "http://h:8123",
            "ai_assist": { "enabled": true },
        }));
        assert!(check_config(&c).is_err());
    }

    #[test]
    fn check_config_errs_when_ai_assist_can_never_run_a_call() {
        let key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        for bad in [
            json!({ "enabled": true, "token_encryption_key": key, "max_concurrent": 0 }),
            json!({ "enabled": true, "token_encryption_key": key, "request_timeout_secs": 0 }),
        ] {
            let c = cfg(json!({
                "database_url": "mysql://h:3306/db",
                "clickhouse_url": "http://h:8123",
                "ai_assist": bad,
            }));
            assert!(
                check_config(&c).is_err(),
                "a zero must stop boot, not hang requests"
            );
        }
    }

    #[test]
    fn check_config_ok_when_ai_assist_is_off_without_a_key() {
        let c = cfg(json!({
            "database_url": "mysql://h:3306/db",
            "clickhouse_url": "http://h:8123",
            "ai_assist": { "enabled": false },
        }));
        assert!(check_config(&c).is_ok());
    }

    #[test]
    fn check_config_errs_on_missing_section() {
        assert!(check_config(&AppConfig::default()).is_err());
    }

    #[test]
    fn check_config_errs_on_empty_database_url() {
        let c = cfg(json!({ "database_url": "", "clickhouse_url": "http://h" }));
        assert!(check_config(&c).is_err());
    }

    #[test]
    fn check_config_errs_on_empty_clickhouse_url() {
        let c = cfg(json!({ "database_url": "mysql://h/db", "clickhouse_url": "" }));
        assert!(check_config(&c).is_err());
    }
}
