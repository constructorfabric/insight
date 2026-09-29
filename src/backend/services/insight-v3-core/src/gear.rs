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

/// The state the alert worker checks against.
struct AlertChecks(Arc<crate::api::AppState>);

impl crate::store::alert_schedule::Checks for AlertChecks {
    fn evaluator(&self) -> crate::domain::alerts::evaluation::Evaluator<'_> {
        self.0
            .alert_evaluator()
            .unwrap_or_else(|| unreachable!("the worker is started only with alerts on"))
    }
}

/// How often the schedule is made to say what the store says again. A rule
/// write that reached the store but not the schedule is put right here.
const RECONCILE_EVERY: std::time::Duration = std::time::Duration::from_mins(5);

/// The schedule as the store says it should be, logged when it was not.
async fn repair_schedule(app: &crate::api::AppState) {
    let (Some(store), Some(schedule)) = (app.alert_store(), app.alert_schedule()) else {
        return;
    };
    let enabled = match store.enabled().await {
        Ok(enabled) => enabled,
        Err(error) => {
            tracing::error!(error = ?error, "the alert rules could not be read to reconcile");
            return;
        }
    };
    match crate::domain::alerts::schedule::reconcile(schedule, &enabled).await {
        Ok(reconciled) if reconciled.removed > 0 => {
            tracing::warn!(
                removed = reconciled.removed,
                "alert schedule held checks for rules that are gone"
            );
        }
        Ok(_) => {}
        Err(error) => tracing::error!(error = ?error, "the alert schedule could not be reconciled"),
    }
}

/// Brings the alert schedule and worker up, after the rest of the state:
/// the schedule is made to say what the store says, then checks start.
async fn start_alerts(
    config: &crate::config::AlertsConfig,
    app: &Arc<crate::api::AppState>,
    cancellation: tokio_util::sync::CancellationToken,
) -> anyhow::Result<()> {
    let (Some(store), Some(schedule)) = (app.alert_store(), app.alert_schedule()) else {
        return Ok(());
    };

    let enabled = store.enabled().await?;
    let reconciled = crate::domain::alerts::schedule::reconcile(schedule, &enabled).await?;
    tracing::info!(
        scheduled = reconciled.scheduled,
        removed = reconciled.removed,
        "alert schedule reconciled with the stored rules"
    );

    let worker = crate::store::alert_schedule::AlertWorker::start(
        &config.redis_url,
        Arc::new(AlertChecks(Arc::clone(app))),
        config.evaluation_concurrency,
        std::time::Duration::from_secs(config.evaluation_lock_secs),
    )
    .await?;
    tracing::info!(
        concurrency = config.evaluation_concurrency,
        "alert worker started"
    );

    let repairing = Arc::clone(app);
    let stop = cancellation.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(RECONCILE_EVERY);
        every.tick().await;
        loop {
            tokio::select! {
                () = stop.cancelled() => break,
                _ = every.tick() => repair_schedule(&repairing).await,
            }
        }
    });

    tokio::spawn(async move {
        cancellation.cancelled().await;
        worker.stop().await;
        tracing::info!("alert worker stopped");
    });

    Ok(())
}

#[async_trait]
impl Gear for InsightV3CoreGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let config: crate::config::GearConfig = ctx.config()?;
        let config = config.validate()?;
        // The definitions are rows read by name and edited in place, so they
        // live in MariaDB rather than beside the data they describe.
        let db = sea_orm::Database::connect(config.database_url()).await?;
        let db_for_alerts = db.clone();
        let definitions = Arc::new(crate::store::definitions::MariaDefinitions::new(db.clone()));
        let datasets = crate::api::Datasets::new(
            Arc::new(crate::store::datasets::MariaDatasets::new(
                db,
                config.dataset_lease(),
            )),
            crate::store::dataset_tables::DatasetTables::new(config.datasets_client()),
            config.datasets_database(),
            config.dataset_preview_rows(),
        );
        let admission = crate::api::admission::IngestAdmission::new(config.ingest_token());
        let chat = crate::chat::ChatClient::new(config.anthropic_token(), config.chat_model());
        let mut app = crate::api::AppState::new(
            crate::domain::query::metric_query::MetricRunner::new(
                config.clickhouse_query_client(),
                crate::domain::query::metric_query::People::new(config.identity_database()),
            ),
            definitions,
            chat,
            crate::store::identity::IdentityClient::new(config.identity_url())?,
            datasets,
            crate::store::catalog::Catalog::new(
                config.clickhouse_query_client(),
                config.clickhouse_database(),
                &config.datasets_database(),
            ),
        );
        if config.alerts().enabled {
            let alerts = config.alerts();
            app = app.with_alerts(crate::api::Alerts {
                store: Arc::new(crate::store::alerts::MariaAlerts::new(
                    db_for_alerts,
                    alerts.notifications_kept_per_rule,
                )),
                schedule: Arc::new(
                    crate::store::alert_schedule::RedisSchedule::connect(&alerts.redis_url).await?,
                ),
                limits: alerts.limits(),
                destinations: alerts.destinations(),
            });
        }
        let app = Arc::new(app);
        let runtime = RuntimeState {
            app: Arc::clone(&app),
            admission,
        };
        self.runtime
            .set(runtime)
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;

        crate::mcp::start(
            config.mcp(),
            crate::mcp::tools::CustomSurfaces::new(Arc::clone(&app)),
            ctx.cancellation_token().child_token(),
        )
        .await?;

        start_alerts(
            config.alerts(),
            &app,
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

    crate::migration::migrate(config.clickhouse(), config.datasets_database()).await?;
    tracing::info!("datasets database migration complete");

    let db = sea_orm::Database::connect(config.database_url()).await?;
    crate::store::definitions::migration::name_the_first_migration(&db).await?;
    <crate::store::definitions::migration::Migrator as sea_orm_migration::MigratorTrait>::up(
        &db, None,
    )
    .await?;
    tracing::info!("definitions migration complete");

    Ok(())
}
