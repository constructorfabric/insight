use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use secrecy::ExposeSecret;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability};

use crate::config::{GearConfig, extract_gear_config};
use crate::{api, infra};

#[derive(Debug, Default)]
#[toolkit::gear(name = "connectors", capabilities = [rest])]
pub(crate) struct ConnectorsGear {
    state: OnceLock<Arc<api::AppState>>,
}

#[async_trait]
impl Gear for ConnectorsGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let config: GearConfig = ctx.config()?;
        config.validate()?;
        let database_url = config.database_url()?;
        let db = infra::db::connect(database_url.expose_secret()).await?;
        infra::db::assert_schema_compatible(&db).await?;

        let mut ch_config =
            insight_clickhouse::Config::new(&config.clickhouse_url, &config.clickhouse_database)
                .with_query_max_threads(4)
                .with_query_max_memory_bytes(1_610_612_736);
        if !config.clickhouse_user.is_empty() {
            ch_config = ch_config.with_auth(
                &config.clickhouse_user,
                config.clickhouse_password.expose_secret(),
            );
        }
        let state = api::AppState {
            db,
            ch: insight_clickhouse::Client::new(ch_config),
            identity: infra::identity::IdentityClient::new(&config.identity_url)?,
        };
        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("connectors gear already initialized"))?;
        tracing::info!("starting connectors gear");
        Ok(())
    }
}

impl RestApiCapability for ConnectorsGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: axum::Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow::anyhow!("connectors gear not initialized"))?;
        Ok(api::register_routes(router, openapi, Arc::clone(state)))
    }
}

pub(crate) async fn run_migrate(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    let config = extract_gear_config(app)?;
    let database_url = config.database_url()?;
    insight_migration::with_migration_session(
        database_url.expose_secret(),
        infra::db::MIGRATION_LOCK,
        infra::db::MIGRATION_LOCK_TIMEOUT,
        |db| async move {
            use sea_orm_migration::MigratorTrait;
            crate::migration::Migrator::up(&db, None).await?;
            Ok(())
        },
    )
    .await?;
    tracing::info!("migrations complete");
    Ok(())
}

pub(crate) fn check_config(app: &toolkit::bootstrap::AppConfig) -> anyhow::Result<()> {
    extract_gear_config(app)?.validate()
}
