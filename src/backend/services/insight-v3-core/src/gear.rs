use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

#[toolkit::gear(name = "insight-v3-core", capabilities = [rest])]
pub struct InsightV3CoreGear {
    runtime: OnceLock<RuntimeState>,
}

impl Default for InsightV3CoreGear {
    fn default() -> Self {
        Self {
            runtime: OnceLock::new(),
        }
    }
}

impl std::fmt::Debug for InsightV3CoreGear {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InsightV3CoreGear")
            .field("initialized", &self.runtime.get().is_some())
            .finish()
    }
}

#[derive(Debug)]
struct RuntimeState {
    app: Arc<crate::api::AppState>,
    admission: crate::api::admission::IngestAdmission,
}

#[async_trait]
impl Gear for InsightV3CoreGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let config: crate::config::GearConfig = ctx.config()?;
        let config = config.validate()?;
        // The definitions are rows read by name and edited in place, so they
        // live in MariaDB rather than beside the data they describe.
        let definitions: Arc<dyn crate::definitions::Definitions> =
            Arc::new(crate::definitions::MariaDefinitions::new(
                sea_orm::Database::connect(config.database_url()).await?,
            ));
        let admission = crate::api::admission::IngestAdmission::new(config.ingest_token());
        let chat = crate::chat::ChatClient::new(config.anthropic_token(), config.chat_model());
        let app = Arc::new(crate::api::AppState::new(
            crate::raw_data::RawDataStore::new(config.clickhouse_client()),
            crate::tables::TableStore::new(config.clickhouse_client()),
            definitions,
            crate::metric_query::MetricRunner::new(
                config.clickhouse_query_client(),
                crate::metric_query::People::new(config.identity_database()),
            ),
            chat,
            crate::identity::IdentityClient::new(config.identity_url())?,
            crate::catalog::Catalog::new(
                config.clickhouse_query_client(),
                config.clickhouse_database(),
            ),
        ));
        let runtime = RuntimeState {
            app: Arc::clone(&app),
            admission,
        };
        self.runtime
            .set(runtime)
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;

        crate::mcp::start(
            config.mcp(),
            crate::mcp::tools::CustomSurfaces::new(app),
            ctx.cancellation_token().child_token(),
        )
        .await?;

        Ok(())
    }
}

impl RestApiCapability for InsightV3CoreGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: axum::Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let runtime = self
            .runtime
            .get()
            .ok_or_else(|| anyhow::anyhow!("insight-v3-core gear not initialized"))?;

        Ok(crate::api::register_routes(
            router,
            openapi,
            runtime.app.clone(),
            runtime.admission.clone(),
        ))
    }
}

pub(crate) async fn run_migrate(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    let config = crate::config::ValidatedConfig::stores_from_app_config(app)?;

    crate::migration::migrate(config.clickhouse()).await?;
    tracing::info!("raw_data migration complete");

    let db = sea_orm::Database::connect(config.database_url()).await?;
    <crate::definitions::migration::Migrator as sea_orm_migration::MigratorTrait>::up(&db, None)
        .await?;
    tracing::info!("definitions migration complete");

    Ok(())
}
