use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamSpec};
use toolkit_canonical_errors::CanonicalError;
use utoipa::ToSchema;

use super::AppState;
use super::definitions::{DefinitionApiError, custom_error};
use super::errors::ApiErrors;
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionLabel};

#[derive(Debug, Deserialize, ToSchema)]
struct DuplicateRequest {
    /// What the copy is called: trimmed, 1 to 128 characters.
    name: String,
}

impl toolkit::api::api_dto::RequestApiDto for DuplicateRequest {}

#[derive(Debug, Serialize)]
struct DuplicateResponse {
    id: String,
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
    let duplicate = OperationBuilder::post("/v1/dashboards/{id}/duplicate")
        .operation_id("insight_v3_core.dashboards.duplicate")
        .summary("Copy a dashboard, its folder and its tags under a fresh id")
        .anonymous()
        .exposed()
        .param(ParamSpec::path("id").description("The id of the dashboard to copy"))
        .json_request::<DuplicateRequest>(openapi, "The copy's name")
        .json_response(StatusCode::CREATED, "The copy's id")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(duplicate_dashboard)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    router.merge(duplicate)
}

async fn duplicate_dashboard(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<DuplicateRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let Json(request) = body.map_err(|error| DefinitionApiError::unreadable_body(&error))?;

    let from = DefinitionId::parse(&id)
        .map_err(|_| DefinitionApiError::malformed_id(DefinitionKind::Dashboard, &id))?;
    let name =
        DefinitionLabel::parse(&request.name).map_err(DefinitionApiError::definition_error)?;
    let to = state
        .surfaces()
        .duplicate(&from, &name)
        .await
        .map_err(custom_error)?;

    let created = DuplicateResponse {
        id: to.into_string(),
    };
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

#[cfg(test)]
mod tests;
