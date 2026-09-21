//! Taking records in: one record, named by the dataset it belongs to.

use std::sync::Arc;

use axum::extract::{Extension, rejection::JsonRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Deserialize;
use toolkit::api::{OpenApiRegistry, OperationBuilder};
use toolkit_canonical_errors::{CanonicalError, Http, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::admission::{self, IngestAdmission};
use crate::domain::dataset_ingest::IngestError;
use crate::domain::definition::DefinitionName;

#[resource_error("gts.cf.insight.insight_v3_core.raw_data.v1~")]
struct RawDataApiError;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct RawDataRequest {
    /// The dataset this record belongs to, which must already be declared.
    dataset: String,
    raw_data: serde_json::Value,
}

impl toolkit::api::api_dto::RequestApiDto for RawDataRequest {}

pub(crate) fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
    admission: IngestAdmission,
) -> Router {
    let api = OperationBuilder::post("/v1/raw-data")
        .operation_id("insight_v3_core.raw_data.ingest")
        .summary("Store raw JSON data")
        .anonymous()
        .exposed()
        .param(admission::instance_token_parameter())
        .json_request::<RawDataRequest>(openapi, "The dataset name and the record")
        .no_content_response(StatusCode::NO_CONTENT, "Raw data stored")
        .error_400(openapi)
        .error_401(openapi)
        .error_404(openapi)
        .error_413(openapi)
        .error_415(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(ingest_raw_data)
        .register(Router::new(), openapi)
        .layer(Extension(state));

    host_router.merge(admission::protect(api, admission))
}

async fn ingest_raw_data(
    Extension(state): Extension<Arc<AppState>>,
    body: Result<Json<RawDataRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let Json(request) = body.map_err(|error| request_rejection(&error))?;
    let dataset =
        DefinitionName::parse(&request.dataset).map_err(|_| not_ready(&request.dataset))?;

    state
        .dataset_ingest()
        .receive(&dataset, &request.raw_data)
        .await
        .map_err(|error| ingest_error(&dataset, error))?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn request_rejection(error: &JsonRejection) -> CanonicalError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return RawDataApiError::invalid_argument()
            .with_field_violation("body", "Request body exceeds the limit", "TOO_LARGE")
            .with_override(Http::status_code(413))
            .create();
    }
    if error.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE {
        return RawDataApiError::invalid_argument()
            .with_field_violation("body", "Content-Type must be application/json", "INVALID")
            .with_override(Http::status_code(415))
            .create();
    }

    RawDataApiError::invalid_argument()
        .with_field_violation(
            "body",
            "Expected a JSON object containing dataset and raw_data",
            "INVALID",
        )
        .create()
}

/// A record can only land in a dataset somebody declared, so a name that is
/// not one, or names nothing ready, is answered the same way.
fn not_ready(named: &str) -> CanonicalError {
    RawDataApiError::not_found(format!(
        "no dataset named `{named}` is ready to take records"
    ))
    .with_resource(named)
    .create()
}

fn ingest_error(dataset: &DefinitionName, error: IngestError) -> CanonicalError {
    match error {
        IngestError::NotReady => not_ready(dataset.as_str()),
        // Not "no such dataset": it is there, it is ready, and it is the
        // wrong kind for this. Saying it is absent would send a sender
        // looking for a name that is right in front of them.
        IngestError::TakesNoRecords => RawDataApiError::failed_precondition()
            .with_precondition_violation(
                "DATASET_OVER_A_RELATION",
                dataset.as_str(),
                format!(
                    "`{}` reads a relation the warehouse builds; records are not sent into it",
                    dataset.as_str()
                ),
            )
            .create(),
        IngestError::Table(crate::store::dataset_tables::DatasetTableError::Timeout) => {
            RawDataApiError::deadline_exceeded("the record could not be stored in time").create()
        }
        IngestError::Unreadable(source) => {
            tracing::error!(error = ?source, "a record could not be read");
            internal_error()
        }
        IngestError::Table(source) => {
            tracing::error!(error = ?source, "a record could not be stored");
            internal_error()
        }
        IngestError::Store(source) => {
            tracing::error!(error = ?source, "the dataset store did not answer");
            internal_error()
        }
    }
}

fn internal_error() -> CanonicalError {
    CanonicalError::internal("the record could not be stored").create()
}

#[cfg(test)]
mod tests;
