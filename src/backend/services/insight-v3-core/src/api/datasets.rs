//! The datasets endpoints: declaring one, reading one, and taking one away.

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, Http, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::domain::dataset_lifecycle::{Broken, DatasetChangeError, Removal};
use crate::domain::dataset_records::PreviewError;
use crate::domain::datasets::Refused;
use crate::domain::definition::{DefinitionName, MAX_PAGE_LIMIT, Page};
use crate::domain::kinds::dataset::state::DatasetState;
use crate::domain::violation::Violation;
use crate::store::dataset_tables::Record;

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

/// One page of the datasets a reader may open.
#[derive(Debug, Serialize, ToSchema)]
struct DatasetNames {
    names: Vec<String>,
    /// Every match, not the page - so a reader can say what is behind it.
    total: u64,
}

/// What a dataset holds, as a reader is shown it: one page of records, how
/// many have arrived in all, and how wide the page was.
#[derive(Debug, Serialize)]
struct DatasetRecords {
    records: Vec<Record>,
    total: u64,
    /// The page size this installation applied. A reader stepping by offset
    /// reads it rather than assuming the limit it asked for was the one used.
    limit: u64,
}

/// What a reader asked one page of records for.
#[derive(Debug, Deserialize)]
struct LookQuery {
    limit: Option<u64>,
    #[serde(default)]
    offset: u64,
    /// A declared field, or `received_at`. Absent means arrival order.
    order_by: Option<String>,
    #[serde(default)]
    direction: Direction,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Direction {
    Asc,
    #[default]
    Desc,
}

/// Everything that would go with this dataset.
#[derive(Debug, Serialize, ToSchema)]
struct DatasetDependents {
    metrics: Vec<String>,
}

/// What a catalogue read asks for.
#[derive(Debug, Deserialize)]
struct Search {
    #[serde(default)]
    q: String,
    limit: Option<u64>,
    offset: Option<u64>,
}

fn query_param(name: &str, param_type: &str, description: &str) -> ParamSpec {
    ParamSpec {
        name: name.to_owned(),
        location: ParamLocation::Query,
        required: false,
        description: Some(description.to_owned()),
        param_type: param_type.to_owned(),
        array: false,
    }
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
        .error_404(openapi)
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
        .error_400(openapi)
        .error_403(openapi)
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
        .error_400(openapi)
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
        .summary("List the datasets a reader may open, or the ones matching ?q=")
        .anonymous()
        .exposed()
        .param(query_param(
            "q",
            "string",
            "Text to look for in a name or in a declaration",
        ))
        .param(query_param(
            "limit",
            "integer",
            &format!("Page size, 1 to {MAX_PAGE_LIMIT}"),
        ))
        .param(query_param("offset", "integer", "Names to skip"))
        .json_response(StatusCode::OK, "One page of names, and how many match")
        .error_400(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_datasets)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)));

    register_reads(
        router.merge(put).merge(get).merge(delete).merge(list),
        openapi,
        state,
    )
}

async fn put_dataset(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = admin_only(&state, &headers).await?;
    let Json(body) = body.map_err(|error| DatasetApiError::unreadable_body(&error))?;
    let name = DefinitionName::parse(&name).map_err(DatasetApiError::definition_error)?;
    tracing::info!(%caller, dataset = name.as_str(), "declaring a dataset");

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

/// One dataset, as a reader may open it.
///
/// A dataset mid-create or mid-removal is not found rather than half shown:
/// its table is not there yet, or is about to go, so nothing on the page
/// behind it would hold.
async fn get_dataset(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    admin_only(&state, &headers).await?;
    let name = DefinitionName::parse(&name).map_err(DatasetApiError::definition_error)?;

    let held = state
        .datasets()
        .get(&name)
        .await
        .map_err(DatasetApiError::dataset_store_error)?
        .filter(|held| held.state == DatasetState::Ready);

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
    let caller = admin_only(&state, &headers).await?;
    let name = DefinitionName::parse(&name).map_err(DatasetApiError::definition_error)?;
    tracing::info!(%caller, dataset = name.as_str(), "removing a dataset");

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
    headers: axum::http::HeaderMap,
    Query(search): Query<Search>,
) -> Result<Response, CanonicalError> {
    admin_only(&state, &headers).await?;
    let page = Page::parse(search.limit, search.offset)
        .map_err(|error| DatasetApiError::invalid_field("limit", error.to_string()))?;

    let found = state
        .datasets()
        .page(search.q.trim(), page)
        .await
        .map_err(DatasetApiError::dataset_store_error)?;

    Ok(Json(DatasetNames {
        names: found.names,
        total: found.total,
    })
    .into_response())
}

/// What a dataset's page reads beside its declaration: the records that have
/// arrived, and what would go with it.
fn register_reads(router: Router, openapi: &dyn OpenApiRegistry, state: &Arc<AppState>) -> Router {
    let name_param = ParamSpec {
        name: "name".to_owned(),
        location: ParamLocation::Path,
        required: true,
        description: Some("Dataset name".to_owned()),
        param_type: "string".to_owned(),
        array: false,
    };

    let records = OperationBuilder::get("/v1/datasets/{name}/records")
        .operation_id("insight_v3_core.datasets.records")
        .summary("One page of the records a dataset holds")
        .anonymous()
        .exposed()
        .param(name_param.clone())
        .param(query_param(
            "limit",
            "integer",
            "How many records this page holds, from 1 to the service's cap",
        ))
        .param(query_param("offset", "integer", "Records to skip"))
        .param(query_param(
            "order_by",
            "string",
            "A declared field, or `received_at`; absent means arrival order",
        ))
        .param(query_param(
            "direction",
            "string",
            "`asc` or `desc`; defaults to `desc`",
        ))
        .json_response(
            StatusCode::OK,
            "This page of records, and how many the dataset holds",
        )
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(dataset_records)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)));

    let dependents = OperationBuilder::get("/v1/datasets/{name}/dependents")
        .operation_id("insight_v3_core.datasets.dependents")
        .summary("Every metric that reads this dataset")
        .anonymous()
        .exposed()
        .param(name_param)
        .json_response(StatusCode::OK, "The metric names")
        .error_400(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(dataset_dependents)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)));

    router.merge(records).merge(dependents)
}

/// The latest records a dataset holds, as they arrived.
async fn dataset_records(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    Query(look): Query<LookQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    admin_only(&state, &headers).await?;
    let name = DefinitionName::parse(&name).map_err(DatasetApiError::definition_error)?;

    let preview = state
        .dataset_records()
        .page(
            &name,
            &crate::domain::dataset_records::Look {
                limit: look.limit,
                offset: look.offset,
                order_by: look.order_by,
                descending: matches!(look.direction, Direction::Desc),
            },
        )
        .await
        .map_err(preview_error)?;

    Ok(Json(DatasetRecords {
        records: preview.records,
        total: preview.total,
        limit: preview.limit,
    })
    .into_response())
}

/// Every metric that reads this dataset, so a reader sees what a removal
/// would take with it before asking for one.
async fn dataset_dependents(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    admin_only(&state, &headers).await?;
    let name = DefinitionName::parse(&name).map_err(DatasetApiError::definition_error)?;

    if crate::domain::datasets::ready(state.datasets(), name.as_str())
        .await
        .map_err(DatasetApiError::dataset_store_error)?
        .is_none()
    {
        return Err(not_found(name.as_str()));
    }

    let metrics = state
        .dataset_lifecycle()
        .dependents(&name)
        .await
        .map_err(change_error)?;

    Ok(Json(DatasetDependents { metrics }).into_response())
}

fn preview_error(error: PreviewError) -> CanonicalError {
    match error {
        PreviewError::NoSuchField { named, declared } => DatasetApiError::invalid_argument()
            .with_field_violation(
                "order_by",
                format!(
                    "`{named}` is not a field of this dataset; it declares {}",
                    declared.join(", ")
                ),
                "UNKNOWN",
            )
            .create(),
        PreviewError::PageSize { asked, cap } => DatasetApiError::invalid_argument()
            .with_field_violation(
                "limit",
                format!("a page holds 1 to {cap} records; {asked} were asked for"),
                "OUT_OF_RANGE",
            )
            .create(),
        PreviewError::NotReady(named) => not_found(&named),
        PreviewError::Relation(source) => {
            tracing::error!(error = ?source, "a dataset's relation could not be read");
            CanonicalError::internal("the dataset's rows could not be reached").create()
        }
        PreviewError::Store(source) => DatasetApiError::dataset_store_error(source),
        PreviewError::Table(source) => {
            tracing::error!(error = ?source, "a dataset's records could not be read");
            CanonicalError::internal("the dataset store did not answer").create()
        }
    }
}

async fn admin_only(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<uuid::Uuid, CanonicalError> {
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
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
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
        DatasetChangeError::Relation(source) => {
            tracing::error!(error = ?source, "the warehouse could not be asked about a relation");
            CanonicalError::internal("the warehouse could not be reached").create()
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
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
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
    let builder = builder.with_override(Http::status_code(StatusCode::CONFLICT.as_u16()));

    builder.create()
}

#[cfg(test)]
mod tests;
