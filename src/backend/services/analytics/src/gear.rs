//! The `analytics` gear.
//!
//! Hosts the analytics REST surface on the `api-gateway` system gear (the REST
//! host) under `toolkit::bootstrap::run_server`. All runtime construction that
//! used to live in `main.rs::run_server` — the self-managed MariaDB pool, its
//! migrations + startup probes, the ClickHouse / Identity clients, the catalog
//! cache, the schema-validator and admin-CRUD service — happens in
//! [`AnalyticsApiGear::init`]. Auth is disabled on this host; the tenant
//! override layer lives in [`crate::auth`].
//!
//! The DB is self-managed (LOCKED DECISION): we do NOT use the toolkit `db`
//! capability — ClickHouse is not a toolkit-db backend, and the gear keeps its
//! own sea-orm pool in `AppState`.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::config::GearConfig;
use crate::domain::admin_threshold::AdminThresholdService;
use crate::domain::auth::ConfigTenantAuthorization;
use crate::domain::catalog::{CatalogReader, ThresholdResolver};
use crate::domain::schema_validator::SchemaValidator;
use crate::infra::cache::catalog_cache::{CatalogCache, NoopCatalogCache, RedisCatalogCache};
use crate::{api, infra};

/// Analytics API gear. Capabilities: `rest` only (the startup schema-validator
/// scan is a one-shot `tokio::spawn` in `init`, faithful to the old
/// `run_server`; no `stateful`/`RunnableCapability`).
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
        tracing::info!("starting analytics gear");

        // Connect to MariaDB (self-managed pool — LOCKED DECISION).
        let db = infra::db::connect(&cfg.database_url).await?;

        // Run pending migrations.
        infra::db::run_migrations(&db).await?;

        // Converge builtin metric definitions to the code registry before
        // serving traffic. MySQL-only, so it does not violate the
        // post-readiness ClickHouse rule; failure aborts startup because the
        // registry state must be consistent before the first request.
        crate::domain::metric_definitions::reconcile_builtin_definitions(&db).await?;

        // Refuse to start if any required CHECK constraint is missing. See
        // `infra/db/check_probe` and DESIGN §2.2
        // `cpt-metric-cat-constraint-mariadb-check`.
        infra::db::check_probe::assert_required_checks(&db).await?;

        // Refuse to start if any enabled `metric_catalog` row is missing its
        // `product-default` `metric_threshold` floor (Refs #523). See
        // `infra/db/product_default_probe` and DESIGN §3.6.
        infra::db::product_default_probe::assert_product_default_present(&db).await?;

        // Catalog cache (Refs #524). Redis when configured; otherwise the
        // no-op stub for single-replica dev installs. Redis-mode boot is
        // best-effort — a Redis blip MUST NOT gate boot.
        let catalog_cache: Arc<dyn CatalogCache> = if cfg.redis_url.is_empty() {
            tracing::info!(
                "catalog_cache: redis_url not configured; using no-op stub. \
                 Multi-replica deploys MUST configure redis_url per \
                 cpt-metric-cat-nfr-cross-replica-invalidation."
            );
            Arc::new(NoopCatalogCache::default())
        } else {
            match RedisCatalogCache::connect(&cfg.redis_url).await {
                Ok(c) => {
                    tracing::info!(
                        redis_url = %redact_url(&cfg.redis_url),
                        "catalog_cache: Redis backend connected"
                    );
                    Arc::new(c)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        redis_url = %redact_url(&cfg.redis_url),
                        "catalog_cache: Redis connection failed at boot; \
                         degrading to no-op stub. Cross-replica invalidation \
                         NFR will not hold until Redis is restored."
                    );
                    Arc::new(NoopCatalogCache::default())
                }
            }
        };

        // Flush the catalog cache so newly seeded rows are visible on the next
        // read without waiting for the TTL. Best-effort.
        if let Err(e) = catalog_cache.flush_all().await {
            tracing::warn!(error = %e, "catalog_cache: flush_all failed at boot; continuing");
        }

        // Threshold resolver + reader (Refs #524).
        let catalog_reader =
            CatalogReader::new(catalog_cache.clone(), ThresholdResolver::new(db.clone()));

        // Connect to ClickHouse.
        let mut ch_config =
            insight_clickhouse::Config::new(&cfg.clickhouse_url, &cfg.clickhouse_database);
        if let (Some(user), Some(password)) = (&cfg.clickhouse_user, &cfg.clickhouse_password) {
            ch_config = ch_config.with_auth(user, password);
        }
        let ch = insight_clickhouse::Client::new(ch_config);

        // Identity client.
        let identity = infra::identity::IdentityClient::new(&cfg.identity_url);

        // Schema-validator (Refs #521). Held in AppState (admin-crud per-write
        // hook) and cloned into the post-init startup pass below.
        let validator = SchemaValidator::new(db.clone(), ch.clone());
        let metric_definition_validator =
            crate::domain::metric_definitions::MetricDefinitionValidator::new(
                db.clone(),
                ch.clone(),
            );
        // Catalog auth-trait (Refs #522 / #525). v1 stub — see `domain::auth`.
        let tenant_auth: Arc<dyn crate::domain::auth::TenantAuthorization> = Arc::new(
            ConfigTenantAuthorization::new(cfg.metric_catalog.tenant_default_id),
        );

        // Admin-CRUD service (Refs #525).
        let admin_threshold = AdminThresholdService::new(
            db.clone(),
            tenant_auth.clone(),
            catalog_cache.clone(),
            validator.clone(),
        );

        let state = api::AppState {
            db,
            ch,
            identity,
            config: cfg,
            validator: validator.clone(),
            tenant_auth,
            catalog_reader,
            admin_threshold,
        };

        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;

        // Startup schema-validator scan (Refs #521). One-shot, post-init, so a
        // ClickHouse outage at boot can never delay readiness — faithful to the
        // old `run_server`'s `tokio::spawn(validator.validate_all())`.
        tokio::spawn(async move {
            validator.validate_all().await;
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

/// `analytics migrate`: run migrations + both startup probes + a
/// best-effort cache flush, then exit. Mirrors the old `main.rs::run_migrate`,
/// reading the gear config out of the loaded `AppConfig` (toolkit owns config
/// layering; the figment loader is gone).
///
/// # Errors
///
/// Returns an error if config extraction, DB connect, migrations, or either
/// probe fails.
pub async fn run_migrate(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    tracing::info!("running migrations");

    let cfg = extract_gear_config(app)?;

    let db = infra::db::connect(&cfg.database_url).await?;
    infra::db::run_migrations(&db).await?;

    // Same convergence as `init`: `migrate` run as a standalone step must
    // leave builtin metric definitions matching the code registry.
    crate::domain::metric_definitions::reconcile_builtin_definitions(&db).await?;

    // Same probes as `init`. An operator running `migrate` standalone wants
    // the integrity signals too (DESIGN §2.2 / §3.6).
    infra::db::check_probe::assert_required_checks(&db).await?;
    infra::db::product_default_probe::assert_product_default_present(&db).await?;

    // DESIGN §3.6 seed sequence ends with `cache_layer.flush_all()`. Operators
    // who run `migrate` as a standalone step need the same flush. Best-effort.
    let catalog_cache: Arc<dyn CatalogCache> = if cfg.redis_url.is_empty() {
        Arc::new(NoopCatalogCache::default())
    } else {
        match RedisCatalogCache::connect(&cfg.redis_url).await {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "catalog_cache: Redis connection failed during migrate; \
                     skipping flush (next server boot will retry)"
                );
                Arc::new(NoopCatalogCache::default())
            }
        }
    };
    if let Err(e) = catalog_cache.flush_all().await {
        tracing::warn!(error = %e, "catalog_cache: flush_all failed after migrate; continuing");
    }

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
    Ok(())
}

/// Redact userinfo (`user:pass@`) from a connection URL before logging, so
/// credentials embedded in e.g. `redis://:pass@host` never reach the logs.
fn redact_url(url: &str) -> String {
    match (url.find("://"), url.find('@')) {
        (Some(scheme_end), Some(at)) if at > scheme_end + 3 => {
            format!("{}{}", &url[..scheme_end + 3], &url[at + 1..])
        }
        _ => url.to_owned(),
    }
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
    fn redact_url_strips_userinfo() {
        assert_eq!(redact_url("redis://:secret@host:6379"), "redis://host:6379");
        assert_eq!(redact_url("redis://user:pw@h:1/0"), "redis://h:1/0");
    }

    #[test]
    fn redact_url_passthrough_without_userinfo() {
        assert_eq!(redact_url("redis://host:6379"), "redis://host:6379");
        assert_eq!(redact_url("not-a-url"), "not-a-url");
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
