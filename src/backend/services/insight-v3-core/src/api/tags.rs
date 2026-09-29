use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, Http, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::domain::definition::DefinitionName;
use crate::domain::tags::{TagError, TagName, TagSet, TagSummary};

#[resource_error("gts.cf.insight.insight_v3_core.tags.v1~")]
struct TagApiError;

impl ApiErrors for TagApiError {
    fn invalid_field(field: &str, detail: String) -> CanonicalError {
        Self::invalid_argument()
            .with_field_violation(field, detail, "INVALID")
            .create()
    }

    fn timed_out(detail: &str) -> CanonicalError {
        Self::deadline_exceeded(detail).create()
    }

    fn name_taken(name: &str) -> CanonicalError {
        Self::already_exists(format!("a tag named `{name}` already exists"))
            .with_resource(name)
            .create()
    }

    fn missing(resource: &str, detail: String) -> CanonicalError {
        Self::not_found(detail).with_resource(resource).create()
    }

    fn oversized(field: &str, detail: &str) -> CanonicalError {
        Self::invalid_argument()
            .with_field_violation(field, detail, "TOO_LARGE")
            .create()
    }
}

#[derive(Debug, Deserialize, ToSchema)]
struct TagsRequest {
    /// Every tag the dashboard carries from now on, each trimmed to 1 to 32
    /// characters, at most 10; `[]` clears them.
    tags: Vec<String>,
}

impl toolkit::api::api_dto::RequestApiDto for TagsRequest {}

pub(crate) fn tags_json(tags: &[TagSummary]) -> Value {
    let tags: Vec<Value> = tags
        .iter()
        .map(|summary| json!({ "name": summary.name.as_str(), "dashboards": summary.dashboards }))
        .collect();

    json!({ "tags": tags })
}

pub(crate) fn names_json(tags: &[TagName]) -> Value {
    json!(tags.iter().map(TagName::as_str).collect::<Vec<_>>())
}

pub(super) fn tags_in(query: Option<&str>) -> Vec<String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .filter(|(key, _)| key == "tag")
        .map(|(_, value)| value.into_owned())
        .collect()
}

pub(super) fn tag_error(error: TagError) -> CanonicalError {
    let detail = error.to_string();

    match error {
        TagError::Name | TagError::TooManyOnDashboard => TagApiError::invalid_field("tags", detail),
        TagError::FilterTooWide | TagError::NotTagged(_) => {
            TagApiError::invalid_field("tag", detail)
        }
        TagError::DashboardNotFound(name) => {
            TagApiError::not_found(detail).with_resource(name).create()
        }
        TagError::TooMany => TagApiError::failed_precondition()
            .with_precondition_violation("tags", detail, "too_many")
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
            .create(),
        TagError::Store(source) => TagApiError::definition_store_error(source),
    }
}

pub(super) fn tag_field_error(error: &TagError) -> CanonicalError {
    TagApiError::invalid_field("tag", error.to_string())
}

fn denied() -> CanonicalError {
    TagApiError::permission_denied()
        .with_reason(crate::api::ADMIN_ONLY)
        .create()
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    let list = OperationBuilder::get("/v1/tags")
        .operation_id("insight_v3_core.tags.list")
        .summary("List the dashboard tags, each with how many dashboards carry it")
        .anonymous()
        .exposed()
        .json_response(StatusCode::OK, "Every tag, sorted by name")
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_tags)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let set = OperationBuilder::put("/v1/dashboards/{name}/tags")
        .operation_id("insight_v3_core.dashboards.tags")
        .summary("Replace the tags a dashboard carries")
        .anonymous()
        .exposed()
        .param(ParamSpec::path("name").description("Dashboard name"))
        .json_request::<TagsRequest>(openapi, "The dashboard's whole set of tags")
        .no_content_response(StatusCode::NO_CONTENT, "Tags set")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(set_tags)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    router.merge(list).merge(set)
}

async fn list_tags(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;

    let listed = state.tags().list_tags().await.map_err(tag_error)?;

    Ok(Json(tags_json(&listed)).into_response())
}

async fn set_tags(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<TagsRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| TagApiError::unreadable_body(&error))?;

    let dashboard = DefinitionName::parse(&name).map_err(TagApiError::definition_error)?;
    let tags = TagSet::parse(&request.tags).map_err(tag_error)?;
    state
        .tags()
        .set_tags(&dashboard, &tags)
        .await
        .map_err(tag_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests;
