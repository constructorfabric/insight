use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::Extension;
use futures::stream::{self, StreamExt};
use serde::de::DeserializeOwned;
use toolkit_canonical_errors::CanonicalError;

use super::AppState;
use super::error::MetricError;
use crate::domain::metric_results::{
    BatchItem, BreakdownQueryRow, CompiledQuery, HistogramQueryRow, MetricResultViewDto,
    MetricResultsRequest, MetricResultsResponse, PeerWideRow, PeriodWideRow, PlannedQuery,
    RankingQueryRow, TimeseriesQueryRow, UnbatchedView, ValidatedMetricResultsRequest,
    build_breakdown_view, build_histogram_view, build_metric_result, build_peer_view,
    build_period_view, build_ranked_groups, build_timeseries_view, demux_peer_rows,
    demux_period_rows, enforce_view_row_limit, plan_queries, plan_rankings, validate_request,
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
    let mut ranking_results = BTreeMap::new();
    let mut rankings = stream::iter(plan_rankings(&req))
        .map(|ranking| {
            let state = Arc::clone(&state);
            async move {
                let comment = format!("metric-results:ranking:{}", ranking.key.rank_metric_key);
                let rows = fetch_rows::<RankingQueryRow>(&state, ranking.query, &comment).await?;
                let groups = build_ranked_groups(&ranking.dimensions, rows)?;
                Ok::<_, CanonicalError>((ranking.key, groups))
            }
        })
        .buffer_unordered(QUERY_CONCURRENCY);
    while let Some(result) = rankings.next().await {
        let (key, groups) = result?;
        ranking_results.insert(key, groups);
    }
    let planned = plan_queries(&req, &ranking_results)?;

    let mut views_by_metric: Vec<Vec<Option<MetricResultViewDto>>> = req
        .metrics
        .iter()
        .map(|metric| (0..metric.views.len()).map(|_| None).collect())
        .collect();

    // Consuming results as they complete bails on the first error; dropping
    // the stream cancels the in-flight and queued queries.
    let mut results = stream::iter(planned)
        .map(|query| execute_planned(&state, &req, query))
        .buffer_unordered(QUERY_CONCURRENCY);
    while let Some(result) = results.next().await {
        for view in result? {
            views_by_metric[view.metric_index][view.view_index] = Some(view.view);
        }
    }

    let mut metrics = Vec::with_capacity(req.metrics.len());
    for (idx, metric) in req.metrics.iter().enumerate() {
        let mut views = Vec::with_capacity(metric.views.len());
        for (view_index, view) in views_by_metric[idx].drain(..).enumerate() {
            let Some(view) = view else {
                return Err(CanonicalError::internal("missing metric view result").create());
            };
            enforce_view_row_limit(&view, format!("metrics[{idx}].views[{view_index}]"))?;
            views.push(view);
        }
        metrics.push(build_metric_result(&metric.def, views));
    }

    let response = MetricResultsResponse { metrics };
    Ok(Json(response))
}

struct MetricViewResult {
    metric_index: usize,
    view_index: usize,
    view: MetricResultViewDto,
}

async fn execute_planned(
    state: &Arc<AppState>,
    req: &ValidatedMetricResultsRequest,
    planned: PlannedQuery,
) -> Result<Vec<MetricViewResult>, CanonicalError> {
    match planned {
        PlannedQuery::PeriodBatch { items, query } => {
            let comment = batch_log_comment("period", &items);
            let rows = fetch_rows::<PeriodWideRow>(state, query, &comment).await?;
            let rows_by_item = demux_period_rows(&items, rows)?;
            Ok(items
                .iter()
                .zip(rows_by_item)
                .map(|(item, rows)| view_result(item, build_period_view(&item.def, req, rows)))
                .collect())
        }
        PlannedQuery::PeerBatch { items, query } => {
            let comment = batch_log_comment("peer", &items);
            let rows = fetch_rows::<PeerWideRow>(state, query, &comment).await?;
            let rows_by_item = demux_peer_rows(&items, rows)?;
            Ok(items
                .iter()
                .zip(rows_by_item)
                .map(|(item, rows)| view_result(item, build_peer_view(rows)))
                .collect())
        }
        PlannedQuery::Single {
            metric_index,
            view_index,
            def,
            view,
            query,
        } => {
            let view = match view {
                UnbatchedView::Timeseries {
                    bucket, dimensions, ..
                } => {
                    let comment = format!("metric-results:timeseries:{}", def.key());
                    let rows = fetch_rows::<TimeseriesQueryRow>(state, query, &comment).await?;
                    build_timeseries_view(&def, req, bucket, &dimensions, rows)?
                }
                UnbatchedView::Breakdown { dimensions } => {
                    let comment = format!("metric-results:breakdown:{}", def.key());
                    let rows = fetch_rows::<BreakdownQueryRow>(state, query, &comment).await?;
                    build_breakdown_view(&dimensions, rows)?
                }
                UnbatchedView::Histogram => {
                    let comment = format!("metric-results:histogram:{}", def.key());
                    let rows = fetch_rows::<HistogramQueryRow>(state, query, &comment).await?;
                    build_histogram_view(req, rows)
                }
            };
            Ok(vec![MetricViewResult {
                metric_index,
                view_index,
                view,
            }])
        }
    }
}

// Batching collapses the per-metric query_log signal; the log_comment keeps
// per-query attribution measurable (`system.query_log.log_comment`).
fn batch_log_comment(kind: &str, items: &[BatchItem]) -> String {
    let keys = items
        .iter()
        .map(|item| item.def.key())
        .collect::<Vec<_>>()
        .join(",");
    format!("metric-results:{kind}-batch:{keys}")
}

fn view_result(item: &BatchItem, view: MetricResultViewDto) -> MetricViewResult {
    MetricViewResult {
        metric_index: item.metric_index,
        view_index: item.view_index,
        view,
    }
}

async fn fetch_rows<T>(
    state: &Arc<AppState>,
    query: CompiledQuery,
    log_comment: &str,
) -> Result<Vec<T>, CanonicalError>
where
    T: DeserializeOwned,
{
    let mut ch_query = state
        .ch
        .query(&query.sql)
        .with_option("log_comment", log_comment);
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
