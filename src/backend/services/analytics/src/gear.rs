use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::config::GearConfig;
use crate::domain::external_links::ExternalSourceRegistry;
use crate::{api, infra};

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
        validate_reports_config(&cfg.reports)?;
        crate::sql_explorer::validate_config(&cfg.mcp, &cfg.sql_api)?;
        let mcp = cfg.mcp.clone();
        let sql_api = cfg.sql_api.clone();
        let mcp_clickhouse_url = cfg.clickhouse_url.clone();
        let mcp_clickhouse_database = cfg.clickhouse_database.clone();
        let report_temp_dir = cfg.reports.temp_dir.clone();
        tokio::task::spawn_blocking(move || {
            crate::domain::reports::temp::prepare_temp_dir(&report_temp_dir)
        })
        .await??;
        let external_links = ExternalSourceRegistry::new(&cfg.external_sources)?;
        tracing::info!("starting analytics gear");

        let db = infra::db::connect(&cfg.database_url).await?;

        infra::db::assert_schema_compatible(&db).await?;

        infra::db::check_probe::assert_required_checks(&db).await?;

        let mut ch_config =
            insight_clickhouse::Config::new(&cfg.clickhouse_url, &cfg.clickhouse_database)
                .with_query_max_threads(4)
                .with_query_max_memory_bytes(1_610_612_736);
        if let (Some(user), Some(password)) = (&cfg.clickhouse_user, &cfg.clickhouse_password) {
            ch_config = ch_config.with_auth(user, password);
        }
        let ch = insight_clickhouse::Client::new(ch_config);

        let identity = infra::identity::IdentityClient::new(&cfg.identity_url)?;

        let metric_definition_validator =
            crate::domain::metric_definitions::MetricDefinitionValidator::new(
                db.clone(),
                ch.clone(),
            );
        let periodic_validation_enabled = cfg.metric_catalog.periodic_validation_enabled;

        let contract_ch = ch.clone();

        let anthropic = infra::anthropic::AnthropicClient::new(
            &cfg.ai_assist.api_base,
            std::time::Duration::from_secs(cfg.ai_assist.request_timeout_secs),
        )?;
        let ai_calls = Arc::new(tokio::sync::Semaphore::new(cfg.ai_assist.max_concurrent));
        let report_generations = Arc::new(tokio::sync::Semaphore::new(
            cfg.reports.max_concurrent_generations,
        ));
        let report_artifacts = Arc::new(tokio::sync::Semaphore::new(
            cfg.reports.max_concurrent_artifacts,
        ));

        let state = api::AppState {
            db,
            ch,
            identity,
            anthropic,
            ai_calls,
            report_generations,
            report_artifacts,
            config: cfg,
            external_links,
        };

        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;

        crate::sql_explorer::start(
            &mcp,
            &sql_api,
            &mcp_clickhouse_url,
            &mcp_clickhouse_database,
            ctx.cancellation_token().child_token(),
        )
        .await?;

        tokio::spawn(async move {
            crate::domain::contract_version::run(&contract_ch).await;
        });
        tokio::spawn(metric_definition_validator.run(periodic_validation_enabled));

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

            crate::domain::metric_definitions::reconcile_builtin_definitions(&db).await?;

            infra::db::check_probe::assert_required_checks(&db).await?;
            Ok(())
        },
    )
    .await?;

    tracing::info!("migrations complete");
    Ok(())
}

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
    crate::sql_explorer::validate_config(&cfg.mcp, &cfg.sql_api)?;
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
    validate_reports_config(&cfg.reports)?;
    Ok(())
}

fn validate_reports_config(cfg: &crate::config::ReportsConfig) -> anyhow::Result<()> {
    crate::domain::reports::temp::validate_temp_dir_path(&cfg.temp_dir)
        .map_err(|message| anyhow::anyhow!("gears.analytics.config.reports.temp_dir {message}"))?;
    for (name, value) in [
        ("max_batch_cells", cfg.max_batch_cells),
        ("max_generated_bytes", cfg.max_generated_bytes),
        ("max_xlsx_spool_bytes", cfg.max_xlsx_spool_bytes),
        ("max_concurrent_generations", cfg.max_concurrent_generations),
        ("max_concurrent_artifacts", cfg.max_concurrent_artifacts),
        ("writer_channel_batches", cfg.writer_channel_batches),
    ] {
        if value == 0 {
            anyhow::bail!("gears.analytics.config.reports.{name} must be at least 1");
        }
    }
    if cfg.max_total_cells == 0 {
        anyhow::bail!("gears.analytics.config.reports.max_total_cells must be at least 1");
    }
    if cfg.request_timeout_secs == 0 || cfg.capacity_wait_secs == 0 {
        anyhow::bail!("gears.analytics.config.reports request timeouts must be at least 1");
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

    #[test]
    fn check_config_rejects_unsafe_or_unbounded_report_storage() {
        for reports in [
            json!({ "temp_dir": "/" }),
            json!({ "temp_dir": "/app/.." }),
            json!({ "max_xlsx_spool_bytes": 0 }),
        ] {
            let c = cfg(json!({
                "database_url": "mysql://h/db",
                "clickhouse_url": "http://h",
                "reports": reports,
            }));

            assert!(check_config(&c).is_err(), "should reject: {reports}");
        }
    }
}
