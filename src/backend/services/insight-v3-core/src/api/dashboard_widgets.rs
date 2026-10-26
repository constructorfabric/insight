//! A dashboard's widgets, drawn through the dashboard: whoever sees the
//! dashboard reads and runs them without reaching the library.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::{Value, json};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, resource_error};

use super::AppState;
use super::definitions::custom_error;
use super::errors::ApiErrors;
use super::{metric_drilldown, metric_run};
use crate::domain::access::Caller;
use crate::domain::definition::{DefinitionId, DefinitionKind};
use crate::domain::surfaces::{CustomError, DrawnWidget, Reading};

pub(crate) const MAX_CONCURRENT_WIDGET_RUNS: usize = 32;
const WIDGET_RUN_ACQUIRE_TIMEOUT_SECS: u64 = 2;
const WIDGET_RUNS_BUSY: &str = "every widget run slot is taken; try again shortly";

#[resource_error("gts.cf.insight.insight_v3_core.dashboard_widgets.v1~")]
struct DashboardWidgetApiError;

impl ApiErrors for DashboardWidgetApiError {
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

/// What a member is told when a widget cannot be drawn; the cause is logged.
const NOT_DRAWN: &str = "this widget could not be drawn";

const NOT_ON_DASHBOARD: &str = "this widget is not on the dashboard";

fn not_drawn() -> CanonicalError {
    CanonicalError::internal(NOT_DRAWN).create()
}

fn readying_error(
    caller: Caller,
    dashboard: &DefinitionId,
    widget: &DefinitionId,
    error: CustomError,
) -> CanonicalError {
    if caller.is_administrator() {
        return custom_error(error);
    }
    if let CustomError::NotFound {
        kind: DefinitionKind::Dashboard,
        ..
    } = error
    {
        return custom_error(error);
    }
    if let CustomError::NotFound {
        kind: DefinitionKind::Widget,
        ..
    } = error
    {
        return DashboardWidgetApiError::not_found(NOT_ON_DASHBOARD)
            .with_resource("widget")
            .create();
    }

    tracing::warn!(
        %error,
        dashboard_id = dashboard.as_str(),
        widget_id = widget.as_str(),
        "a widget could not be readied for a member"
    );
    not_drawn()
}

async fn widget_run_slot(
    state: &AppState,
) -> Result<tokio::sync::OwnedSemaphorePermit, CanonicalError> {
    let slots = state.widget_run_slots();
    let waited = tokio::time::timeout(
        Duration::from_secs(WIDGET_RUN_ACQUIRE_TIMEOUT_SECS),
        Arc::clone(slots).acquire_owned(),
    )
    .await;
    if let Ok(Ok(permit)) = waited {
        return Ok(permit);
    }

    tracing::warn!(
        capacity = MAX_CONCURRENT_WIDGET_RUNS,
        "dashboard widget run capacity exhausted"
    );
    Err(
        DashboardWidgetApiError::resource_exhausted(WIDGET_RUNS_BUSY)
            .with_quota_violation("dashboard widget runs", "concurrency limit reached")
            .with_quota_violation_retry_after_seconds(WIDGET_RUN_ACQUIRE_TIMEOUT_SECS)
            .create(),
    )
}

fn unidentified() -> CanonicalError {
    DashboardWidgetApiError::permission_denied()
        .with_reason(crate::api::UNIDENTIFIED)
        .create()
}

fn denied() -> CanonicalError {
    DashboardWidgetApiError::permission_denied()
        .with_reason(crate::api::ADMIN_ONLY)
        .create()
}

fn ids(dashboard: &str, widget: &str) -> Result<(DefinitionId, DefinitionId), CanonicalError> {
    let dashboard = DefinitionId::parse(dashboard)
        .map_err(|_| DashboardWidgetApiError::malformed_id(DefinitionKind::Dashboard, dashboard))?;
    let widget = DefinitionId::parse(widget)
        .map_err(|_| DashboardWidgetApiError::malformed_id(DefinitionKind::Widget, widget))?;

    Ok((dashboard, widget))
}

fn widget_json(drawn: &DrawnWidget) -> Value {
    json!({
        "id": drawn.id,
        "name": drawn.name,
        "body": drawn.body,
        "clock": drawn.clock,
    })
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    let dashboard = || ParamSpec::path("id").description("Dashboard id");
    let widget = || ParamSpec::path("widget_id").description("Widget id, one the dashboard draws");

    let list = OperationBuilder::get("/v1/dashboards/{id}/widgets")
        .operation_id("insight_v3_core.dashboards.widgets.list")
        .summary("Read the widgets a dashboard the caller sees draws, in its order")
        .anonymous()
        .exposed()
        .param(dashboard())
        .json_response(
            StatusCode::OK,
            "Each widget's id, name, body and the clock of the metric it draws",
        )
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_widgets)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let run = OperationBuilder::post("/v1/dashboards/{id}/widgets/{widget_id}/run")
        .operation_id("insight_v3_core.dashboards.widgets.run")
        .summary("Run the metric a widget of a dashboard the caller sees draws")
        .anonymous()
        .exposed()
        .param(dashboard())
        .param(widget())
        .json_response(StatusCode::OK, "Query result")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(run_widget)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    let drilldown = OperationBuilder::post("/v1/dashboards/{id}/widgets/{widget_id}/drilldown")
        .operation_id("insight_v3_core.dashboards.widgets.drilldown")
        .summary("Read the rows behind a widget of a dashboard, one ordered page at a time: its detail metric, else the metric it draws")
        .anonymous()
        .exposed()
        .param(dashboard())
        .param(widget())
        .json_response(
            StatusCode::OK,
            "One page of rows, and the cursor to the next",
        )
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(drill_widget)
        .register(Router::new(), openapi)
        .layer(Extension(state.clone()));

    router.merge(list).merge(run).merge(drilldown)
}

async fn list_widgets(
    Extension(state): Extension<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::caller(&state, &headers, unidentified).await?;

    let dashboard = DefinitionId::parse(&id)
        .map_err(|_| DashboardWidgetApiError::malformed_id(DefinitionKind::Dashboard, &id))?;
    let drawn = state
        .surfaces(caller)
        .drawn_widgets(&dashboard)
        .await
        .map_err(custom_error)?;

    let items: Vec<Value> = drawn.iter().map(widget_json).collect();
    Ok(Json(json!({ "items": items })).into_response())
}

async fn run_widget(
    Extension(state): Extension<Arc<AppState>>,
    Path((id, widget)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    body: Option<Json<Value>>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::caller(&state, &headers, unidentified).await?;

    let (dashboard, widget) = ids(&id, &widget)?;
    let requested = metric_run::window(body)?;
    let metric = state
        .surfaces(caller)
        .drawn_metric(&dashboard, &widget, Reading::Drawn)
        .await
        .map_err(|error| readying_error(caller, &dashboard, &widget, error))?;

    // INVARIANT: the permit lives until the run answers; it is what caps
    // concurrent widget runs at MAX_CONCURRENT_WIDGET_RUNS.
    let _permit = widget_run_slot(&state).await?;
    match state.metric_runs().run(&metric, &requested).await {
        Ok(result) => Ok(Json(result).into_response()),
        Err(error) if caller.is_administrator() => Err(metric_run::run_error(error)),
        Err(error) => {
            tracing::error!(
                %error,
                dashboard_id = dashboard.as_str(),
                widget_id = widget.as_str(),
                "a widget drawn for a member failed"
            );
            Err(not_drawn())
        }
    }
}

async fn drill_widget(
    Extension(state): Extension<Arc<AppState>>,
    Path((id, widget)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    body: Result<Option<Json<Value>>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let caller = crate::api::require_admin(&state, &headers, denied).await?;

    let (dashboard, widget) = ids(&id, &widget)?;
    let metric = state
        .surfaces(caller)
        .drawn_metric(&dashboard, &widget, Reading::Drilled)
        .await
        .map_err(custom_error)?;

    metric_drilldown::read_page(&state, &metric, body).await
}

#[cfg(test)]
mod tests;
