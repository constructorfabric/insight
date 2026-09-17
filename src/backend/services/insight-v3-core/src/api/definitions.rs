//! Metric, widget and dashboard definition HTTP endpoints.

use std::sync::Arc;

use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::domain::definition::{DefinitionKind, DefinitionName, MAX_PAGE_LIMIT, Page, PageError};
use crate::domain::kinds::metric::answerable::EffectiveClock;
use crate::domain::surfaces::CustomError;
use crate::domain::violation::Violation;

/// The query string on a list: what to look for, in a name or in a body, and
/// which page of the matches to answer with.
#[derive(Debug, Default, Deserialize)]
struct Search {
    #[serde(default)]
    q: String,
    limit: Option<u64>,
    offset: Option<u64>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct RenameRequest {
    /// The name it should answer to from now on.
    to: String,
}

impl toolkit::api::api_dto::RequestApiDto for RenameRequest {}

#[derive(Debug, Serialize, ToSchema)]
struct RenameResponse {
    name: String,
    /// The definitions that pointed at the old name and now point here.
    rewritten: Vec<String>,
}

#[resource_error("gts.cf.insight.insight_v3_core.definitions.v1~")]
pub(super) struct DefinitionApiError;

impl ApiErrors for DefinitionApiError {
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

// `.anonymous()`: these routes trust the gateway to authenticate the
// `__Host-sid` session cookie before forwarding. Must stay off the network
// (see docker-compose.yml's loopback port binding).
pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    DefinitionKind::ALL
        .into_iter()
        .fold(router, |router, kind| {
            register_kind(router, openapi, state.clone(), kind)
        })
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

fn register_list(
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
    kind: DefinitionKind,
    segment: &str,
) -> Router {
    OperationBuilder::get(format!("/v1/{segment}"))
        .operation_id(format!("insight_v3_core.{segment}.list"))
        .summary("List definition names, or the ones matching ?q=")
        .anonymous()
        .exposed()
        .param(query_param(
            "q",
            "string",
            "Text to look for in a name or in a stored body",
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
        .handler(list_definitions)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(kind))
}

fn register_kind(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
    kind: DefinitionKind,
) -> Router {
    let segment = kind.plural();

    let name_param = ParamSpec {
        name: "name".to_owned(),
        location: ParamLocation::Path,
        required: true,
        description: Some("Definition name".to_owned()),
        param_type: "string".to_owned(),
        array: false,
    };

    let delete_param = name_param.clone();
    let rename_param = name_param.clone();

    let put = OperationBuilder::put(format!("/v1/{segment}/{{name}}"))
        .operation_id(format!("insight_v3_core.{segment}.put"))
        .summary("Create or replace a definition")
        .anonymous()
        .exposed()
        .param(name_param.clone())
        .json_request::<serde_json::Value>(openapi, "The definition body")
        .no_content_response(StatusCode::NO_CONTENT, "Definition stored")
        .error_400(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(put_definition)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(kind));

    let get = OperationBuilder::get(format!("/v1/{segment}/{{name}}"))
        .operation_id(format!("insight_v3_core.{segment}.get"))
        .summary("Read a definition")
        .anonymous()
        .exposed()
        .param(name_param)
        .json_response(StatusCode::OK, "The definition body")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(get_definition)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(kind));

    let list = register_list(openapi, &state, kind, segment);

    let remove = OperationBuilder::delete(format!("/v1/{segment}/{{name}}"))
        .operation_id(format!("insight_v3_core.{segment}.delete"))
        .summary("Remove a definition")
        .anonymous()
        .exposed()
        .param(delete_param)
        .no_content_response(StatusCode::NO_CONTENT, "Definition removed")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(delete_definition)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()))
        .layer(Extension(kind));

    let rename = OperationBuilder::post(format!("/v1/{segment}/{{name}}/rename"))
        .operation_id(format!("insight_v3_core.{segment}.rename"))
        .summary("Rename a definition, and everything that points at it")
        .anonymous()
        .exposed()
        .param(rename_param)
        .json_request::<RenameRequest>(openapi, "The new name")
        .json_response(StatusCode::OK, "The new name, and what was rewritten")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(rename_definition)
        .register(Router::new(), openapi)
        .layer(Extension(state))
        .layer(Extension(kind));

    host_router
        .merge(put)
        .merge(get)
        .merge(list)
        .merge(remove)
        .merge(rename)
}

/// A stored definition as a reader is given it.
///
/// The body is what was written; the clock is not in it, because a metric
/// over a dataset may inherit one, and a reader deciding whether to window a
/// card cannot tell from the body alone.
#[derive(Debug, Serialize)]
struct DefinitionResponse {
    body: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    clock: Option<EffectiveClock>,
}

pub(super) fn custom_error(error: CustomError) -> CanonicalError {
    match error {
        CustomError::NotFound { kind, name } => {
            DefinitionApiError::not_found(format!("{} `{name}` was not found", kind.singular()))
                .with_resource(&name)
                .create()
        }
        CustomError::InUse { used_by } => DefinitionApiError::failed_precondition()
            .with_precondition_violation(
                "name",
                format!("still in use by {}", used_by.join(", ")),
                "in_use",
            )
            .create(),
        CustomError::Widget(source) => widget_error(&source),
        CustomError::DatasetNotReady(named) => DefinitionApiError::invalid_field(
            "body",
            format!("no dataset named `{named}` is ready"),
        ),
        CustomError::Body(source) => DefinitionApiError::invalid_field("body", source.to_string()),
        CustomError::Range(source) => {
            DefinitionApiError::invalid_field("time_ranges", source.to_string())
        }
        CustomError::Compile(source) => {
            DefinitionApiError::invalid_field("body", source.to_string())
        }
        CustomError::Unanswerable(violations) => unanswerable(&violations),
        CustomError::Store(source) => DefinitionApiError::definition_store_error(source),
        CustomError::Datasets(source) => DefinitionApiError::dataset_store_error(source),
        CustomError::Run(source) => {
            tracing::error!(error = ?source, "metric query execution failed");
            CanonicalError::internal("metric query execution failed").create()
        }
    }
}

/// Every way the dataset cannot answer the metric, reported together.
///
/// A metric wrong in several places is answered once, with each place named
/// as the submitted body shapes it, so an editor can mark all of them.
fn unanswerable(violations: &[Violation]) -> CanonicalError {
    let Some((first, rest)) = violations.split_first() else {
        return DefinitionApiError::invalid_field(
            "body",
            "the dataset cannot answer this metric".to_owned(),
        );
    };

    let mut builder = DefinitionApiError::invalid_argument().with_field_violation(
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

/// The same body, pointed at the new name.
///
/// Renames a definition, and rewrites whatever drew it under the old name.
async fn rename_definition(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    Json(request): Json<RenameRequest>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, || {
        DefinitionApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;

    let from = DefinitionName::parse(&name).map_err(DefinitionApiError::definition_error)?;
    let to = DefinitionName::parse(&request.to).map_err(DefinitionApiError::definition_error)?;

    let rewritten = state
        .surfaces()
        .rename(kind, &from, &to)
        .await
        .map_err(custom_error)?;

    Ok(Json(RenameResponse {
        name: to.as_str().to_owned(),
        rewritten,
    })
    .into_response())
}

async fn delete_definition(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, || {
        DefinitionApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;

    let name = DefinitionName::parse(&name).map_err(DefinitionApiError::definition_error)?;

    match state.surfaces().delete(kind, &name).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT.into_response()),
        Err(CustomError::NotFound { .. }) => Ok(StatusCode::NOT_FOUND.into_response()),
        Err(other) => Err(custom_error(other)),
    }
}

async fn put_definition(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, || {
        DefinitionApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;

    let name = DefinitionName::parse(&name).map_err(DefinitionApiError::definition_error)?;

    state
        .surfaces()
        .put(kind, &name, &body)
        .await
        .map_err(custom_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn widget_error(error: &crate::domain::kinds::widget::WidgetError) -> CanonicalError {
    DefinitionApiError::invalid_argument()
        .with_field_violation("body", error.to_string(), "INVALID")
        .create()
}

async fn get_definition(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, || {
        DefinitionApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;

    let name = DefinitionName::parse(&name).map_err(DefinitionApiError::definition_error)?;

    let surfaces = state.surfaces();
    match surfaces.get(kind, &name).await {
        Ok(body) => {
            let clock = surfaces.clock_of(kind, &body).await;

            Ok(Json(DefinitionResponse { body, clock }).into_response())
        }
        Err(CustomError::NotFound { .. }) => Ok(StatusCode::NOT_FOUND.into_response()),
        Err(other) => Err(custom_error(other)),
    }
}

async fn list_definitions(
    Extension(state): Extension<Arc<AppState>>,
    Extension(kind): Extension<DefinitionKind>,
    headers: axum::http::HeaderMap,
    Query(search): Query<Search>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, || {
        DefinitionApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;

    let page = Page::parse(search.limit, search.offset).map_err(page_error)?;
    let found = state
        .surfaces()
        .page(kind, &search.q, page)
        .await
        .map_err(custom_error)?;

    Ok(Json(serde_json::json!({
        "names": found.names,
        "total": found.total,
        "limit": page.limit(),
        "offset": page.offset(),
    }))
    .into_response())
}

fn page_error(error: PageError) -> CanonicalError {
    DefinitionApiError::invalid_field("limit", error.to_string())
}

#[cfg(test)]
mod tests;
