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
use crate::custom::{CustomError, held_by};
use crate::definitions::{
    Change, DefinitionError, DefinitionKind, DefinitionName, DefinitionStoreError, MAX_PAGE_LIMIT,
    Page, PageError,
};

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
struct DefinitionApiError;

// `.anonymous()`: these routes trust the gateway to authenticate the
// `__Host-sid` session cookie before forwarding. Must stay off the network
// (see docker-compose.yml's loopback port binding).
pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let router = register_kind(
        router,
        openapi,
        state.clone(),
        DefinitionKind::Metric,
        "metrics",
    );
    let router = register_kind(
        router,
        openapi,
        state.clone(),
        DefinitionKind::Widget,
        "widgets",
    );

    register_kind(
        router,
        openapi,
        state,
        DefinitionKind::Dashboard,
        "dashboards",
    )
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
    segment: &str,
) -> Router {
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

pub(crate) fn custom_error(error: CustomError) -> CanonicalError {
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
        CustomError::Body(source) => DefinitionApiError::invalid_argument()
            .with_field_violation("body", source.to_string(), "INVALID")
            .create(),
        CustomError::Range(source) => DefinitionApiError::invalid_argument()
            .with_field_violation("time_ranges", source.to_string(), "INVALID")
            .create(),
        CustomError::Compile(source) => DefinitionApiError::invalid_argument()
            .with_field_violation("body", source.to_string(), "INVALID")
            .create(),
        CustomError::Store(source) => definition_store_error(source),
        CustomError::Run(source) => {
            tracing::error!(error = ?source, "metric query execution failed");
            CanonicalError::internal("metric query execution failed").create()
        }
        CustomError::Catalog(source) => {
            tracing::error!(error = ?source, "table catalogue read failed");
            CanonicalError::internal("table catalogue read failed").create()
        }
    }
}

/// The same body, pointed at the new name.
///
/// A dashboard also names its widgets inside its item list, where the order
/// and the headings live; nothing else has one, so walking it is a no-op for
/// a widget's metric.
fn pointed_at(body: serde_json::Value, field: &str, from: &str, to: &str) -> serde_json::Value {
    let mut body = crate::dashboard::renamed(body, from, to);

    match body.get_mut(field) {
        Some(serde_json::Value::String(one)) if one == from => to.clone_into(one),
        Some(serde_json::Value::Array(many)) => {
            for entry in many {
                if entry.as_str() == Some(from) {
                    *entry = serde_json::Value::String(to.to_owned());
                }
            }
        }
        _ => {}
    }

    body
}

/// Renames a definition, and rewrites whatever drew it under the old name.
///
/// A name is the only handle a widget has on its metric, and a dashboard on
/// its widgets, so renaming one alone would break the others - the same
/// broken chart the widget check exists to prevent. The new name, the removal
/// of the old, and every rewritten dependent are one transaction.
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

    let from = DefinitionName::parse(&name).map_err(definition_error)?;
    let to = DefinitionName::parse(&request.to).map_err(definition_error)?;

    let body = state
        .definitions()
        .get(kind, &from)
        .await
        .map_err(definition_store_error)?
        .ok_or_else(|| {
            DefinitionApiError::not_found(format!("`{}` was not found", from.as_str()))
                .with_resource(from.as_str())
                .create()
        })?;

    if to == from {
        return Ok(Json(RenameResponse {
            name: to.as_str().to_owned(),
            rewritten: Vec::new(),
        })
        .into_response());
    }

    if state
        .definitions()
        .get(kind, &to)
        .await
        .map_err(definition_store_error)?
        .is_some()
    {
        return Err(DefinitionApiError::already_exists(format!(
            "`{}` is already taken",
            to.as_str()
        ))
        .with_resource(to.as_str())
        .create());
    }

    let mut changes = vec![
        Change::Put(kind, to.clone(), body),
        Change::Delete(kind, from.clone()),
    ];
    let mut rewritten = Vec::new();
    if let Some((holder, field)) = held_by(kind) {
        for holder_name in state
            .surfaces()
            .dependents_of(kind, &from)
            .await
            .map_err(custom_error)?
        {
            let parsed = DefinitionName::parse(&holder_name).map_err(definition_error)?;
            let Some(body) = state
                .definitions()
                .get(holder, &parsed)
                .await
                .map_err(definition_store_error)?
            else {
                continue;
            };

            changes.push(Change::Put(
                holder,
                parsed,
                pointed_at(body, field, from.as_str(), to.as_str()),
            ));
            rewritten.push(holder_name);
        }
    }

    state
        .definitions()
        .apply(&changes)
        .await
        .map_err(definition_store_error)?;

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

    let name = DefinitionName::parse(&name).map_err(definition_error)?;

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

    let name = DefinitionName::parse(&name).map_err(definition_error)?;

    state
        .surfaces()
        .put(kind, &name, &body)
        .await
        .map_err(custom_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

pub(crate) fn widget_error(error: &crate::widget::WidgetError) -> CanonicalError {
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

    let name = DefinitionName::parse(&name).map_err(definition_error)?;

    match state.surfaces().get(kind, &name).await {
        Ok(body) => Ok(Json(body).into_response()),
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
    DefinitionApiError::invalid_argument()
        .with_field_violation("limit", error.to_string(), "INVALID")
        .create()
}

fn definition_error(error: DefinitionError) -> CanonicalError {
    DefinitionApiError::invalid_argument()
        .with_field_violation("name", error.to_string(), "INVALID")
        .create()
}

fn definition_store_error(error: DefinitionStoreError) -> CanonicalError {
    match error {
        // Waiting for a connection is the store being busy, not broken.
        DefinitionStoreError::Database(sea_orm::DbErr::ConnectionAcquire(source)) => {
            tracing::warn!(error = ?source, "definition store connection timed out");
            DefinitionApiError::deadline_exceeded("definition store timed out").create()
        }
        DefinitionStoreError::Database(source) => {
            tracing::error!(error = ?source, "definition store operation failed");
            CanonicalError::internal("definition store operation failed").create()
        }
        DefinitionStoreError::Json(source) => {
            tracing::error!(error = ?source, "definition body serialization failed");
            CanonicalError::internal("definition store operation failed").create()
        }
    }
}

#[cfg(test)]
mod tests;
