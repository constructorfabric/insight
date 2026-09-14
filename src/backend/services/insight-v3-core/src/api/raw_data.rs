//! Raw-data ingestion HTTP endpoint.

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
use crate::raw_data::{RawDataError, RawDataRecord, StoreError};

#[resource_error("gts.cf.insight.insight_v3_core.raw_data.v1~")]
struct RawDataApiError;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct RawDataRequest {
    table: String,
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
        .json_request::<RawDataRequest>(openapi, "Physical table name and raw JSON data")
        .no_content_response(StatusCode::NO_CONTENT, "Raw data stored")
        .error_400(openapi)
        .error_401(openapi)
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
    let record = parse_record(request).await?;

    state
        .raw_data()
        .insert(record)
        .await
        .map_err(|error| store_error(&error))?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn parse_record(request: RawDataRequest) -> Result<RawDataRecord, CanonicalError> {
    let result = tokio::task::spawn_blocking(move || {
        RawDataRecord::parse(&request.table, &request.raw_data)
    })
    .await
    .map_err(|error| {
        tracing::error!(error = ?error, "raw data preparation task failed");
        internal_error()
    })?;

    result.map_err(raw_data_error)
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
            "Expected a JSON object containing table and raw_data",
            "INVALID",
        )
        .create()
}

fn raw_data_error(error: RawDataError) -> CanonicalError {
    match error {
        RawDataError::Table(source) => RawDataApiError::invalid_argument()
            .with_field_violation("table", source.to_string(), "INVALID")
            .create(),
        RawDataError::Serialization(source) => {
            tracing::error!(error = ?source, "raw data serialization failed");
            internal_error()
        }
    }
}

fn store_error(error: &StoreError) -> CanonicalError {
    match error {
        StoreError::Timeout => {
            RawDataApiError::deadline_exceeded("raw data insert timed out").create()
        }
        StoreError::ClickHouse(source) => {
            tracing::error!(error = ?source, "raw data insert failed");
            internal_error()
        }
        StoreError::Create(source) => {
            tracing::error!(error = ?source, "the stream's table could not be created");
            internal_error()
        }
        StoreError::NoTable => {
            tracing::error!("the stream's table is absent after it was created");
            internal_error()
        }
    }
}

fn internal_error() -> CanonicalError {
    CanonicalError::internal("raw data ingestion failed").create()
}

#[cfg(test)]
mod tests;
