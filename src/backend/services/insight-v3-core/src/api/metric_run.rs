//! Compile-and-run endpoint for stored metric definitions.

use std::sync::Arc;

use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, resource_error};

use super::AppState;
use super::errors::ApiErrors;
use crate::domain::definition::DefinitionName;
use crate::domain::query::time_window::WindowRequest;

#[resource_error("gts.cf.insight.insight_v3_core.metric_run.v1~")]
struct MetricRunApiError;

impl ApiErrors for MetricRunApiError {
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
    fn missing(resource: &str, detail: String) -> CanonicalError {
        Self::not_found(detail).with_resource(resource).create()
    }

    fn oversized(field: &str, detail: &str) -> CanonicalError {
        Self::invalid_argument()
            .with_field_violation(field, detail, "TOO_LARGE")
            .create()
    }
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let name_param = ParamSpec {
        name: "name".to_owned(),
        location: ParamLocation::Path,
        required: true,
        description: Some("Metric name".to_owned()),
        param_type: "string".to_owned(),
        array: false,
    };

    let run = OperationBuilder::post("/v1/metrics/{name}/run")
        .operation_id("insight_v3_core.metrics.run")
        .summary("Compile and run a metric")
        .anonymous()
        .exposed()
        .param(name_param)
        .json_response(StatusCode::OK, "Query result")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(run_metric)
        .register(Router::new(), openapi)
        .layer(Extension(state));

    router.merge(run)
}

/// What a caller may ask a run for. An absent body asks for none of it:
/// unbounded, unbucketed and UTC.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RunBody {
    #[serde(default)]
    range: Option<String>,
    #[serde(default)]
    bucket: Option<bool>,
}

async fn run_metric(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    body: Option<Json<serde_json::Value>>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, || {
        MetricRunApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;

    let name = DefinitionName::parse(&name).map_err(MetricRunApiError::definition_error)?;
    let asked = match body {
        Some(Json(value)) => {
            serde_json::from_value::<RunBody>(value).map_err(|error| run_body_error(&error))?
        }
        None => RunBody::default(),
    };

    let requested = WindowRequest::parse(asked.range.as_deref(), asked.bucket)
        .map_err(|error| MetricRunApiError::window_error(&error))?;

    let result = state
        .metric_runs()
        .run(&name, &requested)
        .await
        .map_err(MetricRunApiError::custom_error)?;

    Ok(Json(result).into_response())
}

fn run_body_error(error: &serde_json::Error) -> CanonicalError {
    MetricRunApiError::invalid_argument()
        .with_field_violation("body", error.to_string(), "INVALID")
        .create()
}

#[cfg(test)]
mod tests;
