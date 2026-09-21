//! HTTP API endpoints.

use std::sync::Arc;

use axum::Router;
use toolkit::api::{OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl};

pub(crate) mod admission;
pub(crate) mod chat;
pub(crate) mod datasets;
pub(crate) mod definitions;
mod errors;
pub(crate) mod metric_run;
pub(crate) mod raw_data;

use admission::IngestAdmission;

use crate::chat::ChatClient;
use crate::domain::definition::Definitions;
use crate::domain::query::metric_query::MetricRunner;
use crate::store::dataset_tables::DatasetTables;
use crate::store::identity::IdentityClient;

/// What a refused surface says.
pub(crate) const ADMIN_ONLY: &str = "admin role required for this operation";

/// The caller's own authorization, as the gateway passed it on.
fn forwarded_authorization(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
}

/// Refuses a caller without the admin role.
///
/// The custom surfaces read and write one tenant's definitions and spend the
/// configured model budget, so they are admin-only. Roles live in the identity
/// service, so the caller's authorization is forwarded there.
///
/// An identity that is absent or unreachable is a server error, never a
/// permit: a role check that cannot be made has not passed.
/// Refuses a caller without the admin role, and names the one with it.
pub(crate) async fn require_admin(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    denied: fn() -> toolkit_canonical_errors::CanonicalError,
) -> Result<uuid::Uuid, toolkit_canonical_errors::CanonicalError> {
    if !state.identity.is_configured() {
        tracing::error!("identity service is not configured; admin access cannot be verified");
        return Err(toolkit_canonical_errors::CanonicalError::internal(
            "failed to verify caller permissions",
        )
        .create());
    }

    let caller = state
        .identity
        .admin_caller(forwarded_authorization(headers))
        .await
        .map_err(|error| {
            if error.is_about_the_caller() {
                tracing::warn!(error = %error, "the caller could not be identified");
                return denied();
            }
            tracing::error!(error = %error, "admin role check failed");
            toolkit_canonical_errors::CanonicalError::internal(
                "failed to verify caller permissions",
            )
            .create()
        })?;

    if let Some(caller) = caller {
        return Ok(caller);
    }

    Err(denied())
}

#[derive(Debug)]
pub(crate) struct AppState {
    metrics: MetricRunner,
    definitions: Arc<dyn Definitions>,
    chat: ChatClient,
    identity: IdentityClient,
    datasets: Datasets,
}

/// Everything about datasets this service reaches: their rows, the tables
/// holding their records, and the database those live in.
#[derive(Debug, Clone)]
pub(crate) struct Datasets {
    pub(crate) rows: Arc<dyn crate::domain::datasets::Datasets>,
    pub(crate) tables: Arc<DatasetTables>,
    /// What the warehouse holds, for a declaration over one of its relations
    /// to be checked against. Read-only, and the only handle here that
    /// reaches a database other than the datasets one.
    pub(crate) relations: Arc<crate::store::relations::Relations>,
    pub(crate) database: String,
    /// How many of a dataset's latest records one look shows.
    pub(crate) preview_rows: u64,
}

/// What a state built without configuration shows, which no served request
/// reaches: the openapi dump, and the harnesses.
const DEFAULT_PREVIEW_ROWS: u64 = 50;

impl Datasets {
    pub(crate) fn new(
        rows: Arc<dyn crate::domain::datasets::Datasets>,
        tables: DatasetTables,
        relations: crate::store::relations::Relations,
        database: String,
        preview_rows: u64,
    ) -> Self {
        Self {
            rows,
            tables: Arc::new(tables),
            relations: Arc::new(relations),
            database,
            preview_rows,
        }
    }

    /// Datasets over a store holding datasets that stand ready, for the
    /// harnesses whose metrics have to read something.
    #[cfg(test)]
    pub(crate) fn holding(url: &str, declared: &[(&str, serde_json::Value)]) -> Self {
        use crate::domain::datasets::{Datasets as _, Finish};

        let rows = crate::store::datasets::memory::MemoryDatasets::at(chrono::Utc::now());

        futures::executor::block_on(async {
            for (named, declaration) in declared {
                let name = crate::domain::definition::DefinitionName::parse(named)
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
            crate::store::relations::Relations::new(insight_clickhouse::Client::new(
                insight_clickhouse::Config::new(url, "insight"),
            )),
            "insight_datasets".to_owned(),
            DEFAULT_PREVIEW_ROWS,
        )
    }

    /// Datasets over a store that is never reached, for a state that answers
    /// without one.
    pub(crate) fn offline(url: &str) -> Self {
        Self::new(
            Arc::new(crate::store::datasets::MariaDatasets::new(
                sea_orm::DatabaseConnection::default(),
                crate::domain::datasets::Lease::default(),
            )),
            DatasetTables::new(insight_clickhouse::Client::new(
                insight_clickhouse::Config::new(url, "insight_datasets"),
            )),
            crate::store::relations::Relations::new(insight_clickhouse::Client::new(
                insight_clickhouse::Config::new(url, "insight"),
            )),
            "insight_datasets".to_owned(),
            DEFAULT_PREVIEW_ROWS,
        )
    }
}

impl AppState {
    pub(crate) fn new(
        metrics: MetricRunner,
        definitions: Arc<dyn Definitions>,
        chat: ChatClient,
        identity: IdentityClient,
        datasets: Datasets,
    ) -> Self {
        Self {
            metrics,
            definitions,
            chat,
            identity,
            datasets,
        }
    }

    pub(crate) fn definitions(&self) -> &dyn Definitions {
        self.definitions.as_ref()
    }

    pub(crate) fn metrics(&self) -> &MetricRunner {
        &self.metrics
    }

    pub(crate) fn chat(&self) -> &ChatClient {
        &self.chat
    }

    pub(crate) fn surfaces(&self) -> crate::domain::surfaces::Surfaces<'_> {
        crate::domain::surfaces::Surfaces::new(
            self.definitions.as_ref(),
            self.datasets(),
            &self.datasets.database,
            self.metrics.people(),
        )
    }

    pub(crate) fn assistant(&self) -> crate::domain::assistant::Assistant<'_> {
        crate::domain::assistant::Assistant::new(self.definitions.as_ref(), self.datasets())
    }

    pub(crate) fn datasets(&self) -> &dyn crate::domain::datasets::Datasets {
        self.datasets.rows.as_ref()
    }

    pub(crate) fn dataset_records(&self) -> crate::domain::dataset_records::DatasetRecords<'_> {
        crate::domain::dataset_records::DatasetRecords::new(
            self.datasets.rows.as_ref(),
            &self.datasets.tables,
            &self.datasets.relations,
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
            &self.datasets.relations,
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
}

/// Title, version and description of the emitted document. Kept in step with
/// the api-gateway block of `helm/templates/configmap.yaml`, which declares
/// the same three for the served document.
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

/// The `OpenAPI` document, built offline for the drift gate.
///
/// Every operation is declared while the router is assembled, so the document
/// comes from the code that serves the routes rather than a copy kept beside
/// it. Nothing here dials anything: the `ClickHouse` clients hold URLs they
/// never call, and the definition store holds a disconnected handle — a route
/// reached on this path would answer an error, and none is reached.
///
/// # Errors
/// The registry could not assemble the document.
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
    // Built apart from the host's router so the layer wraps this service's
    // own routes, and merged in after — the host's own endpoints carry
    // their own context.
    let api = raw_data::register_routes(Router::new(), openapi, state.clone(), admission);
    let api = definitions::register_routes(api, openapi, &state);
    let api = datasets::register_routes(api, openapi, &state);
    let api = metric_run::register_routes(api, openapi, state.clone());
    let api = chat::register_routes(api, openapi, state)
        .layer(insight_log_context::LogContextLayer::new());

    router.merge(api)
}

#[cfg(test)]
mod log_context_tests;
#[cfg(test)]
mod log_leak_tests;
#[cfg(test)]
mod openapi_tests;
