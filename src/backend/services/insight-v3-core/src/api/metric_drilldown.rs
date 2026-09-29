//! The rows behind a metric, one ordered page at a time.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::domain::definition::DefinitionName;
use crate::domain::drilldown::{
    Asked, Column, CursorError, DEFAULT_PAGE_ROWS, DrilldownError, MAX_PAGE_ROWS, Page, Sort,
};
use crate::domain::query::metric_query::ColumnKind;
use crate::domain::query::time_window::WindowRequest;

/// How many pages may be read at once, and how long a caller waits for a
/// turn before being told to come back. A page re-runs the metric, so the
/// cap is what keeps a board full of open dialogs from starving the runs
/// the cards themselves make.
pub(crate) const MAX_CONCURRENT_PAGES: usize = 8;
const ACQUIRE_TIMEOUT_SECS: u64 = 2;

#[resource_error("gts.cf.insight.insight_v3_core.metric_drilldown.v1~")]
struct DrilldownApiError;

impl ApiErrors for DrilldownApiError {
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
    let page = OperationBuilder::post("/v1/metrics/{name}/drilldown")
        .operation_id("insight_v3_core.metrics.drilldown")
        .summary("Read a metric's rows one ordered page at a time")
        .anonymous()
        .exposed()
        .param(ParamSpec {
            name: "name".to_owned(),
            location: ParamLocation::Path,
            required: true,
            description: Some("Metric name".to_owned()),
            param_type: "string".to_owned(),
            array: false,
        })
        .json_request::<DrilldownBody>(openapi, "How the page is ordered, cut and continued")
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
        .handler(page_metric)
        .register(Router::new(), openapi)
        .layer(Extension(state));

    router.merge(page)
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
enum Direction {
    Asc,
    Desc,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
struct SortBody {
    key: String,
    direction: Direction,
}

/// What a caller may ask a page for. An absent body asks for the first page
/// of the whole result, in the metric's own order.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct DrilldownBody {
    #[serde(default)]
    range: Option<String>,
    #[serde(default)]
    bucket: Option<bool>,
    #[serde(default)]
    sort: Option<SortBody>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

impl toolkit::api::api_dto::RequestApiDto for DrilldownBody {}

/// The read as the service performed it. `sort` is always the effective
/// order, never the caller's omission.
#[derive(Debug, Serialize, ToSchema)]
struct Selection {
    metric: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bucket: Option<bool>,
    sort: SortBody,
}

#[derive(Debug, Serialize, ToSchema)]
struct ColumnOut {
    key: String,
    label: String,
    r#type: &'static str,
    sortable: bool,
    percent: bool,
}

#[derive(Debug, Serialize, ToSchema)]
struct RowOut {
    values: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
struct DrilldownResponse {
    selection: Selection,
    columns: Vec<ColumnOut>,
    rows: Vec<RowOut>,
    next_cursor: Option<String>,
}

async fn page_metric(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Option<Json<serde_json::Value>>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, || {
        DrilldownApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;

    let name = DefinitionName::parse(&name).map_err(DrilldownApiError::definition_error)?;
    let asked = asked(body)?;

    // INVARIANT: held across the read. The permit is this request's share of
    // MAX_CONCURRENT_PAGES, and the read is what it bounds.
    let _permit = acquire(&state).await?;
    let page = state
        .drilldowns()
        .page(&name, &asked)
        .await
        .map_err(|error| drilldown_error(error, &name))?;

    Ok(Json(response(name.as_str(), &asked, page)).into_response())
}

/// What the caller asked for, read only once they have proved they may ask.
fn asked(
    body: Result<Option<Json<serde_json::Value>>, JsonRejection>,
) -> Result<Asked, CanonicalError> {
    let body = body.map_err(|error| DrilldownApiError::unreadable_body(&error))?;
    let written = match body {
        Some(Json(value)) => serde_json::from_value::<DrilldownBody>(value)
            .map_err(|error| DrilldownApiError::invalid_field("body", error.to_string()))?,
        None => DrilldownBody::default(),
    };
    let window = WindowRequest::parse(written.range.as_deref(), written.bucket)
        .map_err(|error| DrilldownApiError::window_error(&error))?;

    Ok(Asked {
        window,
        range: written.range,
        bucket: written.bucket,
        sort: written.sort.map(|sort| Sort {
            key: sort.key,
            descending: matches!(sort.direction, Direction::Desc),
        }),
        limit: written.limit.unwrap_or(DEFAULT_PAGE_ROWS),
        cursor: written.cursor,
    })
}

async fn acquire(state: &AppState) -> Result<tokio::sync::OwnedSemaphorePermit, CanonicalError> {
    let slots = state.drilldown_slots();
    tokio::time::timeout(
        Duration::from_secs(ACQUIRE_TIMEOUT_SECS),
        Arc::clone(slots).acquire_owned(),
    )
    .await
    .map_err(|_| {
        tracing::warn!(
            capacity = MAX_CONCURRENT_PAGES,
            available = slots.available_permits(),
            "metric drilldown capacity exhausted"
        );
        busy()
    })?
    .map_err(|_| busy())
}

fn busy() -> CanonicalError {
    DrilldownApiError::resource_exhausted("every drilldown slot is taken; try again shortly")
        .with_quota_violation("metric drilldown pages", "concurrency limit reached")
        .with_quota_violation_retry_after_seconds(ACQUIRE_TIMEOUT_SECS)
        .create()
}

fn response(metric: &str, asked: &Asked, page: Page) -> DrilldownResponse {
    DrilldownResponse {
        selection: Selection {
            metric: metric.to_owned(),
            range: asked.range.clone(),
            bucket: asked.bucket,
            sort: SortBody {
                key: page.sort.key,
                direction: if page.sort.descending {
                    Direction::Desc
                } else {
                    Direction::Asc
                },
            },
        },
        columns: page.columns.iter().map(column_out).collect(),
        rows: page
            .rows
            .into_iter()
            .map(|values| RowOut { values })
            .collect(),
        next_cursor: page.next_cursor,
    }
}

fn column_out(column: &Column) -> ColumnOut {
    ColumnOut {
        key: column.key.clone(),
        label: column.key.clone(),
        r#type: match column.kind {
            ColumnKind::Text => "string",
            ColumnKind::Number => "number",
            ColumnKind::Date => "date",
        },
        sortable: true,
        percent: column.percent,
    }
}

fn drilldown_error(error: DrilldownError, name: &DefinitionName) -> CanonicalError {
    let said = error.to_string();

    match error {
        DrilldownError::Custom(source) => DrilldownApiError::custom_error(source),
        DrilldownError::Sort(column) => DrilldownApiError::invalid_field(
            "sort.key",
            format!("`{column}` is not a column of `{}`", name.as_str()),
        ),
        DrilldownError::PageSize => DrilldownApiError::invalid_field(
            "limit",
            format!("a page holds between 1 and {MAX_PAGE_ROWS} rows"),
        ),
        DrilldownError::Cursor(
            CursorError::Malformed | CursorError::Version | CursorError::Selection,
        ) => DrilldownApiError::invalid_field("cursor", said),
        DrilldownError::Rebuilt => DrilldownApiError::failed_precondition()
            .with_precondition_violation(
                "metric source",
                "The table behind this metric was made again while it was being read; start again.",
                "SOURCE_REBUILT",
            )
            .create(),
        DrilldownError::Reserved(_) => DrilldownApiError::invalid_field("body", said),
    }
}

#[cfg(test)]
mod tests;
