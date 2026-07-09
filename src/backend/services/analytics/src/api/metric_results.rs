use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::Extension;
use futures::stream::{self, StreamExt};
use serde::de::DeserializeOwned;
use toolkit_canonical_errors::CanonicalError;

use super::AppState;
use super::error::MetricError;
use crate::domain::metric_definitions::MetricDefinition;
use crate::domain::metric_results::{
    BreakdownQueryRow, CompiledQuery, MetricResultViewDto, MetricResultsRequest,
    MetricResultsResponse, PeerQueryRow, PeriodQueryRow, TimeseriesQueryRow,
    ValidatedMetricResultsRequest, ValidatedMetricView, build_breakdown_view, build_metric_result,
    build_peer_view, build_period_view, build_timeseries_view, compile_view_query,
    enforce_row_limit, validate_request,
};
use toolkit_security::SecurityContext;

const QUERY_CONCURRENCY: usize = 4;
// Client-side bound on one view query, network stalls included. The
// insight-clickhouse client already caps server-side execution at 30s
// (`max_execution_time`); this covers the transport path that setting
// cannot reach (dead peer, half-open connection).
const QUERY_FETCH_TIMEOUT: Duration = Duration::from_mins(1);

pub async fn query_metric_results(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<MetricResultsRequest>,
) -> Result<Json<MetricResultsResponse>, CanonicalError> {
    let req = validate_request(&state.db, ctx.subject_tenant_id(), req).await?;
    let tasks = compile_tasks(&req);

    let mut views_by_metric: Vec<Vec<Option<MetricResultViewDto>>> = req
        .metrics
        .iter()
        .map(|metric| (0..metric.views.len()).map(|_| None).collect())
        .collect();

    // Consuming results as they complete bails on the first error; dropping
    // the stream cancels the in-flight and queued view queries.
    let mut results = stream::iter(tasks)
        .map(|task| execute_task(&state, &req, task))
        .buffer_unordered(QUERY_CONCURRENCY);
    while let Some(result) = results.next().await {
        let result = result?;
        views_by_metric[result.metric_index][result.view_index] = Some(result.view);
    }

    let mut metrics = Vec::with_capacity(req.metrics.len());
    for (idx, metric) in req.metrics.iter().enumerate() {
        let mut views = Vec::with_capacity(metric.views.len());
        for view in views_by_metric[idx].drain(..) {
            let Some(view) = view else {
                return Err(CanonicalError::internal("missing metric view result").create());
            };
            views.push(view);
        }
        metrics.push(build_metric_result(&metric.def, views));
    }

    let response = MetricResultsResponse { metrics };
    enforce_row_limit(&response)?;
    Ok(Json(response))
}

struct MetricViewTask {
    metric_index: usize,
    view_index: usize,
    def: MetricDefinition,
    view: ValidatedMetricView,
    query: CompiledQuery,
}

struct MetricViewTaskResult {
    metric_index: usize,
    view_index: usize,
    view: MetricResultViewDto,
}

fn compile_tasks(req: &ValidatedMetricResultsRequest) -> Vec<MetricViewTask> {
    req.metrics
        .iter()
        .enumerate()
        .flat_map(|(metric_index, metric)| {
            metric
                .views
                .iter()
                .enumerate()
                .map(move |(view_index, view)| MetricViewTask {
                    metric_index,
                    view_index,
                    def: metric.def.clone(),
                    view: view.clone(),
                    query: compile_view_query(&metric.def, req, view),
                })
        })
        .collect()
}

async fn execute_task(
    state: &Arc<AppState>,
    req: &ValidatedMetricResultsRequest,
    task: MetricViewTask,
) -> Result<MetricViewTaskResult, CanonicalError> {
    let MetricViewTask {
        metric_index,
        view_index,
        def,
        view,
        query,
    } = task;

    let view = match view {
        ValidatedMetricView::Period => {
            let rows = fetch_rows::<PeriodQueryRow>(state, query).await?;
            build_period_view(&def, req, rows)
        }
        ValidatedMetricView::Peer { .. } => {
            let rows = fetch_rows::<PeerQueryRow>(state, query).await?;
            build_peer_view(rows)
        }
        ValidatedMetricView::Timeseries { bucket, dimensions } => {
            let rows = fetch_rows::<TimeseriesQueryRow>(state, query).await?;
            build_timeseries_view(&def, req, bucket, &dimensions, rows)?
        }
        ValidatedMetricView::Breakdown { dimensions } => {
            let rows = fetch_rows::<BreakdownQueryRow>(state, query).await?;
            build_breakdown_view(&dimensions, rows)?
        }
    };

    Ok(MetricViewTaskResult {
        metric_index,
        view_index,
        view,
    })
}

async fn fetch_rows<T>(
    state: &Arc<AppState>,
    query: CompiledQuery,
) -> Result<Vec<T>, CanonicalError>
where
    T: DeserializeOwned,
{
    let mut ch_query = state.ch.query(&query.sql);
    for param in &query.params {
        ch_query = ch_query.bind(param.as_str());
    }

    let mut cursor = ch_query.fetch_bytes("JSONEachRow").map_err(|e| {
        tracing::error!(error = %e, sql = %query.sql, "ClickHouse metric-results query failed");
        map_query_error(&e.to_string())
    })?;

    let raw_bytes = tokio::time::timeout(QUERY_FETCH_TIMEOUT, cursor.collect())
        .await
        .map_err(|_| {
            tracing::error!(sql = %query.sql, "ClickHouse metric-results fetch timed out");
            CanonicalError::internal("query execution failed").create()
        })?
        .map_err(|e| {
            tracing::error!(error = %e, sql = %query.sql, "ClickHouse metric-results fetch failed");
            map_query_error(&e.to_string())
        })?;

    if raw_bytes.is_empty() {
        return Ok(Vec::new());
    }

    raw_bytes
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            tracing::error!(error = %e, "failed to parse metric-results rows");
            CanonicalError::internal("failed to parse query results").create()
        })
}

// A missing observation/cohort relation is a known transient state (dbt has
// not built the view yet, or a model regressed) that the validator sweep
// converges on — surface it as a typed precondition failure instead of a
// 500. UNKNOWN_TABLE is ClickHouse error code 60.
fn map_query_error(message: &str) -> CanonicalError {
    if message.contains("UNKNOWN_TABLE") || message.contains("Code: 60") {
        return MetricError::failed_precondition()
            .with_precondition_violation(
                "metric source relation",
                "The observation or cohort view backing this metric has not been built yet; it converges on the next validation sweep.",
                "SOURCE_RELATION_MISSING",
            )
            .create();
    }
    CanonicalError::internal("query execution failed").create()
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::map_query_error;

    #[test]
    fn missing_relation_maps_to_precondition_failure_not_500() {
        let err = map_query_error(
            "bad response: Code: 60. DB::Exception: Table insight.ai_metric_observations does not exist. (UNKNOWN_TABLE)",
        );
        let status = err.into_response().status();
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(status.is_client_error());
    }

    #[test]
    fn other_query_errors_stay_internal() {
        let err = map_query_error("Code: 241. DB::Exception: Memory limit exceeded");
        let status = err.into_response().status();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
