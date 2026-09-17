//! The datasets endpoints: declaring one, reading one, and taking one away.

use std::sync::Arc;

use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Serialize;
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::domain::dataset_lifecycle::{Broken, DatasetChangeError, Removal};
use crate::domain::datasets::Refused;
use crate::domain::definition::DefinitionName;
use crate::domain::violation::Violation;

#[resource_error("gts.cf.insight.insight_v3_core.datasets.v1~")]
struct DatasetApiError;

impl ApiErrors for DatasetApiError {
    fn invalid_field(field: &str, detail: String) -> CanonicalError {
        Self::invalid_argument()
            .with_field_violation(field, detail, "INVALID")
            .create()
    }

    fn timed_out(detail: &str) -> CanonicalError {
        Self::deadline_exceeded(detail).create()
    }

    fn name_taken(name: &str) -> CanonicalError {
        Self::already_exists(format!("`{name}` is already taken"))
            .with_resource(name)
            .create()
    }
}

/// What a reader is told about one dataset.
#[derive(Debug, Serialize, ToSchema)]
struct DatasetResponse {
    name: String,
    /// The declaration as it is stored.
    declaration: serde_json::Value,
}

/// Every dataset there is, whatever state it is in.
#[derive(Debug, Serialize, ToSchema)]
struct DatasetNames {
    names: Vec<String>,
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    let name_param = ParamSpec {
        name: "name".to_owned(),
        location: ParamLocation::Path,
        required: true,
        description: Some("Dataset name".to_owned()),
        param_type: "string".to_owned(),
        array: false,
    };

    let put = OperationBuilder::put("/v1/datasets/{name}")
        .operation_id("insight_v3_core.datasets.put")
        .summary("Declare a dataset, or replace its declaration")
        .anonymous()
        .exposed()
        .param(name_param.clone())
        .json_request::<serde_json::Value>(openapi, "The declaration")
        .json_response(StatusCode::OK, "The stored declaration")
        .error_400(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(put_dataset)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)));

    let get = OperationBuilder::get("/v1/datasets/{name}")
        .operation_id("insight_v3_core.datasets.get")
        .summary("Read a dataset's declaration")
        .anonymous()
        .exposed()
        .param(name_param.clone())
        .json_response(StatusCode::OK, "The stored declaration")
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(get_dataset)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)));

    let delete = OperationBuilder::delete("/v1/datasets/{name}")
        .operation_id("insight_v3_core.datasets.delete")
        .summary("Take a dataset away, with the records it holds")
        .anonymous()
        .exposed()
        .param(name_param)
        .no_content_response(StatusCode::NO_CONTENT, "The dataset is gone")
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(delete_dataset)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)));

    let list = OperationBuilder::get("/v1/datasets")
        .operation_id("insight_v3_core.datasets.list")
        .summary("Every dataset there is")
        .anonymous()
        .exposed()
        .json_response(StatusCode::OK, "The dataset names")
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_datasets)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)));

    router.merge(put).merge(get).merge(delete).merge(list)
}

async fn put_dataset(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, CanonicalError> {
    admin_only(&state, &headers).await?;
    let name = DefinitionName::parse(&name).map_err(DatasetApiError::definition_error)?;

    let stored = state
        .dataset_lifecycle()
        .declare(&name, &body)
        .await
        .map_err(change_error)?;

    Ok(Json(DatasetResponse {
        name: name.as_str().to_owned(),
        declaration: stored,
    })
    .into_response())
}

async fn get_dataset(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Response, CanonicalError> {
    let name = DefinitionName::parse(&name).map_err(DatasetApiError::definition_error)?;

    let held = state
        .datasets()
        .get(&name)
        .await
        .map_err(DatasetApiError::dataset_store_error)?;

    let Some(held) = held else {
        return Err(not_found(name.as_str()));
    };

    Ok(Json(DatasetResponse {
        name: held.name.as_str().to_owned(),
        declaration: held.declaration,
    })
    .into_response())
}

async fn delete_dataset(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    admin_only(&state, &headers).await?;
    let name = DefinitionName::parse(&name).map_err(DatasetApiError::definition_error)?;

    match state
        .dataset_lifecycle()
        .remove(&name)
        .await
        .map_err(change_error)?
    {
        Removal::Removed | Removal::AlreadyUnderWay => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

async fn list_datasets(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Response, CanonicalError> {
    let names = state
        .datasets()
        .list()
        .await
        .map_err(DatasetApiError::dataset_store_error)?;

    Ok(Json(DatasetNames { names }).into_response())
}

async fn admin_only(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<(), CanonicalError> {
    crate::api::require_admin(state, headers, || {
        DatasetApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await
}

fn not_found(name: &str) -> CanonicalError {
    DatasetApiError::not_found(format!("dataset `{name}` was not found"))
        .with_resource(name)
        .create()
}

fn change_error(error: DatasetChangeError) -> CanonicalError {
    match error {
        DatasetChangeError::Invalid(violations) => invalid(&violations),
        DatasetChangeError::Unreadable(source) => {
            DatasetApiError::invalid_field("body", source.to_string())
        }
        DatasetChangeError::StillRead(readers) => DatasetApiError::failed_precondition()
            .with_precondition_violation(
                "name",
                format!("still read by {}", readers.join(", ")),
                "in_use",
            )
            .create(),
        DatasetChangeError::WouldBreak(broken) => would_break(&broken),
        DatasetChangeError::Refused(Refused::Busy(operation)) => DatasetApiError::aborted(format!(
            "a {operation} of this dataset is already under way"
        ))
        .with_reason("OPERATION_UNDER_WAY")
        .create(),
        DatasetChangeError::NotFound | DatasetChangeError::Refused(Refused::Gone) => {
            not_found("the dataset asked for")
        }
        DatasetChangeError::Refused(Refused::Removing) => {
            DatasetApiError::aborted("this name belongs to a removal until it finishes".to_owned())
                .with_reason("OPERATION_UNDER_WAY")
                .create()
        }
        DatasetChangeError::Table(source) => {
            tracing::error!(error = ?source, "a dataset's table could not be reached");
            CanonicalError::internal("the dataset's records could not be reached").create()
        }
        DatasetChangeError::Store(source) => DatasetApiError::dataset_store_error(source),
        DatasetChangeError::Definitions(source) => DatasetApiError::definition_store_error(source),
    }
}

/// Every problem with the submitted declaration, each against the place in the
/// body that carries it.
///
/// A body wrong in several places is answered once, with all of them, so it is
/// corrected in one pass.
fn invalid(violations: &[Violation]) -> CanonicalError {
    let Some((first, rest)) = violations.split_first() else {
        return DatasetApiError::invalid_field("body", "the declaration is not valid".to_owned());
    };

    let mut builder = DatasetApiError::invalid_argument().with_field_violation(
        &first.field,
        first.detail.clone(),
        first.reason_code(),
    );
    for violation in rest {
        builder = builder.with_field_violation(
            &violation.field,
            violation.detail.clone(),
            violation.reason_code(),
        );
    }

    builder.create()
}

/// Every metric this replacement would not leave as it found it.
fn would_break(broken: &[Broken]) -> CanonicalError {
    let Some((first, rest)) = broken.split_first() else {
        return DatasetApiError::failed_precondition()
            .with_precondition_violation("name", "a metric reads this dataset", "would_break")
            .create();
    };

    let mut builder = DatasetApiError::failed_precondition().with_precondition_violation(
        &first.metric,
        first.why.clone(),
        "would_break",
    );
    for one in rest {
        builder = builder.with_precondition_violation(&one.metric, one.why.clone(), "would_break");
    }

    builder.create()
}

#[cfg(test)]
mod tests;
