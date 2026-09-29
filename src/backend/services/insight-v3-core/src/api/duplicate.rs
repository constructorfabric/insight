use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::CanonicalError;
use utoipa::ToSchema;

use super::AppState;
use super::definitions::{DefinitionApiError, custom_error};
use super::errors::ApiErrors;
use crate::domain::definition::DefinitionName;

#[derive(Debug, Deserialize, ToSchema)]
struct DuplicateRequest {
    name: String,
}

impl toolkit::api::api_dto::RequestApiDto for DuplicateRequest {}

#[derive(Debug, Serialize)]
struct DuplicateResponse {
    name: String,
}

fn denied() -> CanonicalError {
    DefinitionApiError::permission_denied()
        .with_reason(crate::api::ADMIN_ONLY)
        .create()
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    let duplicate = OperationBuilder::post("/v1/dashboards/{name}/duplicate")
        .operation_id("insight_v3_core.dashboards.duplicate")
        .summary("Copy a dashboard, its folder and its tags under a new name")
        .anonymous()
        .exposed()
        .param(ParamSpec {
            name: "name".to_owned(),
            location: ParamLocation::Path,
            required: true,
            description: Some("The dashboard to copy".to_owned()),
            param_type: "string".to_owned(),
            array: false,
        })
        .json_request::<DuplicateRequest>(openapi, "The copy's name")
        .json_response(StatusCode::CREATED, "The copy's name")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(duplicate_dashboard)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    router.merge(duplicate)
}

async fn duplicate_dashboard(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<DuplicateRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| DefinitionApiError::unreadable_body(&error))?;

    let from = DefinitionName::parse(&name).map_err(DefinitionApiError::definition_error)?;
    let to = DefinitionName::parse(&request.name).map_err(DefinitionApiError::definition_error)?;
    state
        .surfaces()
        .duplicate(&from, &to)
        .await
        .map_err(custom_error)?;

    let created = DuplicateResponse {
        name: to.into_string(),
    };
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

#[cfg(test)]
mod tests;
