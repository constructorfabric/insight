use std::sync::Arc;

use axum::Router;
use toolkit::api::{OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl};

pub(crate) mod admission;
pub(crate) mod alerts;
pub(crate) mod chat;
pub(crate) mod dashboard_widgets;
pub(crate) mod datasets;
pub(crate) mod definitions;
pub(crate) mod duplicate;
mod errors;
pub(crate) mod folders;
pub(crate) mod metric_drilldown;
pub(crate) mod metric_run;
pub(crate) mod pins;
pub(crate) mod raw_data;
pub(crate) mod sharing;
pub(crate) mod tables;
pub(crate) mod tags;

use admission::IngestAdmission;

use crate::chat::ChatClient;
use crate::domain::access::Caller;
use crate::domain::alerts::delivery::{Deliveries, Provider};
use crate::domain::alerts::rule::AlertStore;
use crate::domain::alerts::schedule::AlertSchedule;
use crate::domain::alerts::{Destinations, Limits};
use crate::domain::folders::{DefinitionStore, Folders};
use crate::domain::pins::Pins;
use crate::domain::query::metric_query::MetricRunner;
use crate::domain::tags::Tags;
use crate::store::catalog::Catalog;
use crate::store::dataset_tables::DatasetTables;
use crate::store::identity::IdentityClient;

pub(crate) const ADMIN_ONLY: &str = "admin role required for this operation";

pub(crate) const UNIDENTIFIED: &str = "the caller could not be identified";

pub(crate) fn forwarded_authorization(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
}

pub(crate) async fn caller(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    denied: fn() -> toolkit_canonical_errors::CanonicalError,
) -> Result<Caller, toolkit_canonical_errors::CanonicalError> {
    if !state.identity.is_configured() {
        tracing::error!("identity service is not configured; the caller cannot be verified");
        return Err(toolkit_canonical_errors::CanonicalError::internal(
            "failed to verify caller permissions",
        )
        .create());
    }

    let caller = state
        .identity
        .caller(forwarded_authorization(headers))
        .await
        .map_err(|error| {
            if error.is_about_the_caller() {
                tracing::warn!(error = %error, "the caller could not be identified");
                return denied();
            }
            tracing::error!(error = %error, "the caller's role could not be read");
            toolkit_canonical_errors::CanonicalError::internal(
                "failed to verify caller permissions",
            )
            .create()
        })?;

    caller.ok_or_else(denied)
}

pub(crate) async fn require_admin(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    denied: fn() -> toolkit_canonical_errors::CanonicalError,
) -> Result<Caller, toolkit_canonical_errors::CanonicalError> {
    let caller = caller(state, headers, denied).await?;
    if caller.is_administrator() {
        return Ok(caller);
    }

    Err(denied())
}

#[derive(Debug)]
pub(crate) struct AppState {
    metrics: MetricRunner,
    definitions: Arc<dyn DefinitionStore>,
    chat: ChatClient,
    identity: IdentityClient,
    datasets: Datasets,
    catalog: Catalog,
    drilldown_slots: Arc<tokio::sync::Semaphore>,
    preview_slots: Arc<tokio::sync::Semaphore>,
    widget_run_slots: Arc<tokio::sync::Semaphore>,
    alerts: Option<Alerts>,
}

#[derive(Debug)]
pub(crate) struct Alerts {
    pub(crate) store: Arc<dyn AlertStore>,
    pub(crate) schedule: Arc<dyn AlertSchedule>,
    pub(crate) deliveries: Arc<dyn Deliveries>,
    pub(crate) providers: std::collections::BTreeMap<String, Arc<dyn Provider>>,
    pub(crate) limits: Limits,
    pub(crate) destinations: Destinations,
}

#[derive(Debug, Clone)]
pub(crate) struct Datasets {
    pub(crate) rows: Arc<dyn crate::domain::datasets::Datasets>,
    pub(crate) tables: Arc<DatasetTables>,
    pub(crate) database: String,
    pub(crate) preview_rows: u64,
}

const DEFAULT_PREVIEW_ROWS: u64 = 50;

impl Datasets {
    pub(crate) fn new(
        rows: Arc<dyn crate::domain::datasets::Datasets>,
        tables: DatasetTables,
        database: String,
        preview_rows: u64,
    ) -> Self {
        Self {
            rows,
            tables: Arc::new(tables),
            database,
            preview_rows,
        }
    }

    #[cfg(test)]
    pub(crate) fn holding(url: &str, declared: &[(&str, serde_json::Value)]) -> Self {
        use crate::domain::datasets::{Datasets as _, Finish};

        let rows = crate::store::datasets::memory::MemoryDatasets::at(chrono::Utc::now());

        futures::executor::block_on(async {
            for (named, declaration) in declared {
                let name = crate::domain::definition::DefinitionId::parse(named)
                    .unwrap_or_else(|error| panic!("`{named}` should be a dataset name: {error}"));
                let attempt = rows
                    .take_create(&name, declaration)
                    .await
                    .unwrap_or_else(|error| panic!("the dataset is claimed: {error}"))
                    .attempt();
                for written in [Finish::Provisioned(format!("ds_{named}_1")), Finish::Ready] {
                    rows.finish(&name, &attempt.token, written)
                        .await
                        .unwrap_or_else(|error| panic!("the dataset is published: {error}"));
                }
            }
        });

        Self::new(
            Arc::new(rows),
            DatasetTables::new(insight_clickhouse::Client::new(
                insight_clickhouse::Config::new(url, "insight_datasets"),
            )),
            "insight_datasets".to_owned(),
            DEFAULT_PREVIEW_ROWS,
        )
    }

    pub(crate) fn offline(url: &str) -> Self {
        Self::new(
            Arc::new(crate::store::datasets::MariaDatasets::new(
                sea_orm::DatabaseConnection::default(),
                crate::domain::datasets::Lease::default(),
            )),
            DatasetTables::new(insight_clickhouse::Client::new(
                insight_clickhouse::Config::new(url, "insight_datasets"),
            )),
            "insight_datasets".to_owned(),
            DEFAULT_PREVIEW_ROWS,
        )
    }
}

impl AppState {
    pub(crate) fn new(
        metrics: MetricRunner,
        definitions: Arc<dyn DefinitionStore>,
        chat: ChatClient,
        identity: IdentityClient,
        datasets: Datasets,
        catalog: Catalog,
    ) -> Self {
        Self {
            metrics,
            definitions,
            chat,
            identity,
            datasets,
            catalog,
            drilldown_slots: Arc::new(tokio::sync::Semaphore::new(
                metric_drilldown::MAX_CONCURRENT_PAGES,
            )),
            preview_slots: Arc::new(tokio::sync::Semaphore::new(alerts::MAX_CONCURRENT_PREVIEWS)),
            widget_run_slots: Arc::new(tokio::sync::Semaphore::new(
                dashboard_widgets::MAX_CONCURRENT_WIDGET_RUNS,
            )),
            alerts: None,
        }
    }

    pub(crate) fn with_alerts(mut self, alerts: Alerts) -> Self {
        self.alerts = Some(alerts);
        self
    }

    pub(crate) fn alerts(&self) -> Option<crate::domain::alerts::rules::AlertRules<'_>> {
        let alerts = self.alerts.as_ref()?;

        Some(crate::domain::alerts::rules::AlertRules::new(
            alerts.store.as_ref(),
            alerts.schedule.as_ref(),
            self.metric_runs(),
            alerts.limits,
            &alerts.destinations,
        ))
    }

    pub(crate) fn alert_store(&self) -> Option<&dyn AlertStore> {
        self.alerts.as_ref().map(|alerts| alerts.store.as_ref())
    }

    pub(crate) fn alert_schedule(&self) -> Option<&dyn AlertSchedule> {
        self.alerts.as_ref().map(|alerts| alerts.schedule.as_ref())
    }

    pub(crate) fn alert_deliveries(&self) -> Option<&dyn Deliveries> {
        self.alerts
            .as_ref()
            .map(|alerts| alerts.deliveries.as_ref())
    }

    pub(crate) fn alert_deliverer(&self) -> Option<crate::domain::alerts::delivery::Deliverer<'_>> {
        let alerts = self.alerts.as_ref()?;

        Some(crate::domain::alerts::delivery::Deliverer::new(
            alerts.store.as_ref(),
            &alerts.providers,
        ))
    }

    pub(crate) fn alert_evaluator(
        &self,
    ) -> Option<crate::domain::alerts::evaluation::Evaluator<'_>> {
        let alerts = self.alerts.as_ref()?;

        Some(crate::domain::alerts::evaluation::Evaluator::new(
            alerts.store.as_ref(),
            self.metric_runs(),
        ))
    }

    pub(crate) fn preview_slots(&self) -> &Arc<tokio::sync::Semaphore> {
        &self.preview_slots
    }

    pub(crate) fn drilldown_slots(&self) -> &Arc<tokio::sync::Semaphore> {
        &self.drilldown_slots
    }

    pub(crate) fn widget_run_slots(&self) -> &Arc<tokio::sync::Semaphore> {
        &self.widget_run_slots
    }

    #[cfg(test)]
    pub(crate) fn definitions(&self) -> &dyn crate::domain::definition::Definitions {
        self.definitions.as_ref()
    }

    pub(crate) fn folders(&self) -> &dyn Folders {
        self.definitions.as_ref()
    }

    pub(crate) fn tags(&self) -> &dyn Tags {
        self.definitions.as_ref()
    }

    pub(crate) fn pins(&self) -> &dyn Pins {
        self.definitions.as_ref()
    }

    pub(crate) fn store(&self) -> &dyn DefinitionStore {
        self.definitions.as_ref()
    }

    pub(crate) fn identity(&self) -> &IdentityClient {
        &self.identity
    }

    pub(crate) fn metrics(&self) -> &MetricRunner {
        &self.metrics
    }

    pub(crate) fn chat(&self) -> &ChatClient {
        &self.chat
    }

    pub(crate) fn surfaces(&self, caller: Caller) -> crate::domain::surfaces::Surfaces<'_> {
        crate::domain::surfaces::Surfaces::new(
            self.definitions.as_ref(),
            self.datasets(),
            &self.datasets.database,
            self.metrics.people(),
            caller,
        )
    }

    pub(crate) fn assistant(&self) -> crate::domain::assistant::Assistant<'_> {
        crate::domain::assistant::Assistant::new(self.definitions.as_ref(), self.datasets())
    }

    pub(crate) fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub(crate) fn datasets(&self) -> &dyn crate::domain::datasets::Datasets {
        self.datasets.rows.as_ref()
    }

    pub(crate) fn dataset_records(&self) -> crate::domain::dataset_records::DatasetRecords<'_> {
        crate::domain::dataset_records::DatasetRecords::new(
            self.datasets.rows.as_ref(),
            &self.datasets.tables,
            self.datasets.preview_rows,
        )
    }

    pub(crate) fn dataset_ingest(&self) -> crate::domain::dataset_ingest::DatasetIngest<'_> {
        crate::domain::dataset_ingest::DatasetIngest::new(
            self.datasets.rows.as_ref(),
            &self.datasets.tables,
        )
    }

    pub(crate) fn dataset_lifecycle(
        &self,
    ) -> crate::domain::dataset_lifecycle::DatasetLifecycle<'_> {
        crate::domain::dataset_lifecycle::DatasetLifecycle::new(
            self.datasets.rows.as_ref(),
            &self.datasets.tables,
            self.definitions.as_ref(),
        )
    }

    pub(crate) fn metric_runs(&self) -> crate::domain::metric_run::MetricRuns<'_> {
        crate::domain::metric_run::MetricRuns::new(
            self.definitions.as_ref(),
            &self.metrics,
            self.datasets.rows.as_ref(),
            &self.datasets.database,
        )
    }

    pub(crate) fn drilldowns(&self) -> crate::domain::drilldown::Drilldowns<'_> {
        crate::domain::drilldown::Drilldowns::new(self.metric_runs(), &self.metrics)
    }
}

fn openapi_info() -> OpenApiInfo {
    OpenApiInfo {
        title: "Insight v3 Core API".to_owned(),
        version: "0.1.0".to_owned(),
        description: Some(
            "Raw-data ingestion under a static instance token, and the \
             metric, widget and dashboard definitions the portal's custom \
             pages read. The API Gateway mounts this service at /api/v3."
                .to_owned(),
        ),
        servers: Vec::new(),
    }
}

pub(crate) fn openapi_document() -> anyhow::Result<utoipa::openapi::OpenApi> {
    let offline = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        "http://clickhouse.invalid",
        "insight",
    ));
    let state = Arc::new(AppState::new(
        MetricRunner::new(
            offline,
            crate::domain::query::metric_query::People::new("identity"),
        ),
        Arc::new(crate::store::definitions::MariaDefinitions::new(
            sea_orm::DatabaseConnection::default(),
        )),
        ChatClient::keyless(),
        IdentityClient::new("http://identity.invalid")?,
        crate::api::Datasets::offline("http://offline.invalid"),
        Catalog::new(
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                "http://clickhouse.invalid",
                "insight",
            )),
            "insight",
            "insight_datasets",
        ),
    ));

    let openapi = OpenApiRegistryImpl::new();
    let _ = register_routes(
        Router::new(),
        &openapi,
        state,
        IngestAdmission::new(&secrecy::SecretString::from(
            "openapi-document-token-not-a-credential".to_owned(),
        )),
    );

    openapi.build_openapi(&openapi_info()).map_err(|error| {
        anyhow::anyhow!("failed to build insight-v3-core OpenAPI document: {error}")
    })
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
    admission: IngestAdmission,
) -> Router {
    let api = raw_data::register_routes(Router::new(), openapi, state.clone(), admission);
    let api = definitions::register_routes(api, openapi, &state);
    let api = duplicate::register_routes(api, openapi, &state);
    let api = sharing::register_routes(api, openapi, &state);
    let api = dashboard_widgets::register_routes(api, openapi, &state);
    let api = folders::register_routes(api, openapi, &state);
    let api = tags::register_routes(api, openapi, &state);
    let api = pins::register_routes(api, openapi, &state);
    let api = datasets::register_routes(api, openapi, &state);
    let api = tables::register_routes(api, openapi, &state);
    let api = metric_run::register_routes(api, openapi, state.clone());
    let api = metric_drilldown::register_routes(api, openapi, state.clone());
    let api = alerts::register_routes(api, openapi, &state);
    let api = chat::register_routes(api, openapi, state)
        .layer(insight_http_metrics::ServerMetricsLayer::new(
            "insight-v3-core",
        ))
        .layer(insight_log_context::LogContextLayer::new());

    router.merge(api)
}

#[cfg(test)]
mod log_context_tests;
#[cfg(test)]
mod log_leak_tests;
#[cfg(test)]
mod openapi_tests;
