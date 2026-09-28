use std::sync::Arc;

use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::json;
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, Http, resource_error};

use super::AppState;
use super::errors::ApiErrors;
use crate::domain::definition::DefinitionName;
use crate::domain::pins::PinError;

#[resource_error("gts.cf.insight.insight_v3_core.pins.v1~")]
struct PinApiError;

impl ApiErrors for PinApiError {
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

fn pin_error(error: PinError) -> CanonicalError {
    let detail = error.to_string();

    match error {
        PinError::DashboardNotFound(name) => {
            PinApiError::not_found(detail).with_resource(name).create()
        }
        PinError::TooMany => PinApiError::failed_precondition()
            .with_precondition_violation("name", detail, "too_many")
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
            .create(),
        PinError::Store(source) => PinApiError::definition_store_error(source),
    }
}

fn denied() -> CanonicalError {
    PinApiError::permission_denied()
        .with_reason(crate::api::ADMIN_ONLY)
        .create()
}

fn name_param() -> ParamSpec {
    ParamSpec {
        name: "name".to_owned(),
        location: ParamLocation::Path,
        required: true,
        description: Some("Dashboard name".to_owned()),
        param_type: "string".to_owned(),
        array: false,
    }
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    let list = OperationBuilder::get("/v1/pins")
        .operation_id("insight_v3_core.pins.list")
        .summary("List the dashboards the caller pinned")
        .anonymous()
        .exposed()
        .json_response(
            StatusCode::OK,
            "The caller's pinned dashboard names, oldest pin first",
        )
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_pins)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let pin = OperationBuilder::put("/v1/pins/{name}")
        .operation_id("insight_v3_core.pins.pin")
        .summary("Pin a dashboard for the caller; a pinned one keeps its place")
        .anonymous()
        .exposed()
        .param(name_param())
        .no_content_response(StatusCode::NO_CONTENT, "Pinned")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(pin_dashboard)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let unpin = OperationBuilder::delete("/v1/pins/{name}")
        .operation_id("insight_v3_core.pins.unpin")
        .summary("Unpin a dashboard for the caller, whether or not it was pinned")
        .anonymous()
        .exposed()
        .param(name_param())
        .no_content_response(StatusCode::NO_CONTENT, "Not pinned")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(unpin_dashboard)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    router.merge(list).merge(pin).merge(unpin)
}

async fn list_pins(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;

    let pins = state.pins().pins_of(caller).await.map_err(pin_error)?;

    Ok(Json(json!({ "pins": pins })).into_response())
}

async fn pin_dashboard(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;

    let dashboard = DefinitionName::parse(&name).map_err(PinApiError::definition_error)?;
    state
        .pins()
        .pin(caller, &dashboard)
        .await
        .map_err(pin_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn unpin_dashboard(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;

    let dashboard = DefinitionName::parse(&name).map_err(PinApiError::definition_error)?;
    state
        .pins()
        .unpin(caller, &dashboard)
        .await
        .map_err(pin_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests;
