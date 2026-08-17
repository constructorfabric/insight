use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::Extension;
use axum::http::HeaderMap;
use futures::stream::{self, StreamExt};
use serde::de::DeserializeOwned;
use toolkit_canonical_errors::CanonicalError;

use super::AppState;
use super::error::MetricError;
use crate::domain::metric_drilldown::load_capabilities;
use crate::domain::metric_results::{
    BatchItem, BreakdownQueryRow, CacheOutcome, CachePlan, CompiledQuery, HistogramQueryRow,
    MetricResultViewDto, MetricResultsRequest, MetricResultsResponse, PeerWideRow, PeriodWideRow,
    PlannedQuery, RankedGroup, RankingPolicyKey, RankingQueryRow, TimeseriesQueryRow,
    UnbatchedView, ValidatedMetricResultsRequest, ViewOutcome, build_breakdown_view,
    build_histogram_view, build_metric_result, build_ranked_groups, build_timeseries_view,
    demux_peer_rows, demux_period_rows, derive_view_keys, enforce_view_row_limit, flat_keys,
    plan_queries, plan_rankings, relation_epochs, required_relations, uncacheable_keys,
    validate_request,
};
use crate::domain::person_visibility::authorize_entity_ids;
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
    headers: HeaderMap,
    Json(req): Json<MetricResultsRequest>,
) -> Result<Json<MetricResultsResponse>, CanonicalError> {
    let tenant_id = ctx.subject_tenant_id();
    let mut req = validate_request(&state.db, tenant_id, req).await?;
    req.enforce_tenant_scope = state.config.metric_catalog.enforce_tenant_scope;

    // Visibility gate BEFORE any ClickHouse work: the caller may only query
    // persons inside their visible set (identity /v1/visible-persons, by
    // person UUID since the cutover). Service principals bypass.
    authorize_entity_ids(
        &state.identity,
        &ctx,
        super::forwarded_authorization(&headers),
        req.entity.entity_type(),
        req.entity.person_ids(),
    )
    .await?;

    let metric_keys = req
        .metrics
        .iter()
        .map(|metric| metric.def.key().to_owned())
        .collect::<Vec<_>>();
    let capabilities = load_capabilities(&state.db, tenant_id, &metric_keys);
    let (plan, capabilities, rankings) = tokio::join!(
        probe_cache(&state, &req),
        capabilities,
        fetch_rankings(&state, &req)
    );

    let mut views_by_metric = resolve_views(&state, &req, plan, rankings?).await?;

    let capabilities = match capabilities {
        Ok(capabilities) => capabilities,
        Err(error) => {
            tracing::warn!(error = ?error, "metric drilldown capability load failed");
            HashMap::default()
        }
    };
    let mut metrics = Vec::with_capacity(req.metrics.len());
    for (idx, metric) in req.metrics.iter().enumerate() {
        let mut views = Vec::with_capacity(metric.views.len());
        for view in views_by_metric[idx].drain(..) {
            let Some(view) = view else {
                return Err(CanonicalError::internal("missing metric view result").create());
            };
            views.push(view);
        }
        let selection = crate::domain::metric_results::MetricResultSelectionDto {
            metric_key: metric.def.key().to_owned(),
            entity: crate::domain::metric_results::MetricResultsEntityDto {
                r#type: req.entity.entity_type().to_owned(),
                ids: req.entity.entity_ids(),
            },
            period: crate::domain::metric_results::MetricResultsPeriodDto {
                from: req.from.to_string(),
                to: req.to.to_string(),
            },
            filters: metric
                .filters
                .iter()
                .map(
                    |filter| crate::domain::metric_results::MetricDimensionFilterDto {
                        dimension: filter.dimension.clone(),
                        values: filter.values.clone(),
                    },
                )
                .collect(),
        };

        let mut result = build_metric_result(&metric.def, views, selection);
        result.drilldown = capabilities.get(metric.def.key()).cloned();
        metrics.push(result);
    }

    let response = MetricResultsResponse { metrics };
    Ok(Json(response))
}

/// Reads the cache for every requested view and reports what is left to run.
/// Anything that goes wrong here — no Redis, no epoch, a slow read — resolves to
/// "nothing was cached".
async fn probe_cache(state: &Arc<AppState>, req: &ValidatedMetricResultsRequest) -> CachePlan {
    if !state.view_cache.enabled() {
        return CachePlan::build(req, uncacheable_keys(req), &[]);
    }

    let relations = required_relations(&req.metrics);
    let Some(epochs) = relation_epochs(&state.ch, &relations).await else {
        return CachePlan::build(req, uncacheable_keys(req), &[]);
    };

    let keys = derive_view_keys(req, &epochs);
    let cached = state.view_cache.get_many(&flat_keys(&keys)).await;

    CachePlan::build(req, keys, &cached)
}

async fn resolve_views(
    state: &Arc<AppState>,
    req: &ValidatedMetricResultsRequest,
    mut plan: CachePlan,
    rankings: BTreeMap<RankingPolicyKey, Vec<RankedGroup>>,
) -> Result<Vec<Vec<Option<MetricResultViewDto>>>, CanonicalError> {
    let cache_stats = plan.stats();
    tracing::debug!(
        hits = cache_stats.hits,
        misses = cache_stats.misses,
        uncacheable = cache_stats.uncacheable,
        "metric-results view cache"
    );

    // Each narrowed request carries its own entity set, so its queries must be
    // built AND rendered against that request, never the caller's.
    let mut jobs = Vec::new();
    for (origin, narrowed) in plan.narrowed() {
        let scope = Arc::new(narrowed.req.clone());
        for query in plan_queries(&narrowed.req, &rankings)? {
            jobs.push((origin, Arc::clone(&scope), query));
        }
    }

    // Consuming results as they complete bails on the first error; dropping
    // the stream cancels the in-flight and queued queries.
    let mut results = stream::iter(jobs)
        .map(|(origin, scope, query)| async move {
            let produced = execute_planned(state, &scope, query).await?;
            Ok::<_, CanonicalError>((origin, produced))
        })
        .buffer_unordered(QUERY_CONCURRENCY);
    while let Some(result) = results.next().await {
        let (origin, produced) = result?;
        for item in produced {
            plan.record(origin, item.metric_index, item.view_index, item.outcome)?;
        }
    }

    let CacheOutcome { views, writes } = plan.finish(req)?;

    // The response gate runs before anything is stored, so a view the caller is
    // about to be refused never reaches Redis.
    for (metric_index, metric_views) in views.iter().enumerate() {
        for (view_index, view) in metric_views.iter().enumerate() {
            let Some(view) = view else { continue };
            enforce_view_row_limit(view, format!("metrics[{metric_index}].views[{view_index}]"))?;
        }
    }

    if let Some(permit) = (!writes.is_empty())
        .then(|| state.view_cache.try_admit_write())
        .flatten()
    {
        let cache = Arc::clone(&state.view_cache);
        tokio::spawn(async move { cache.set_many(permit, writes).await });
    }

    Ok(views)
}

async fn fetch_rankings(
    state: &Arc<AppState>,
    req: &ValidatedMetricResultsRequest,
) -> Result<BTreeMap<RankingPolicyKey, Vec<RankedGroup>>, CanonicalError> {
    let mut ranking_results = BTreeMap::new();
    let mut rankings = stream::iter(plan_rankings(req))
        .map(|ranking| {
            let state = Arc::clone(state);
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

    Ok(ranking_results)
}

struct MetricViewResult {
    metric_index: usize,
    view_index: usize,
    outcome: ViewOutcome,
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
                .map(|(item, rows)| view_result(item, ViewOutcome::PeriodRows(rows)))
                .collect())
        }
        PlannedQuery::PeerBatch { items, query } => {
            let comment = batch_log_comment("peer", &items);
            let rows = fetch_rows::<PeerWideRow>(state, query, &comment).await?;
            let rows_by_item = demux_peer_rows(&items, rows)?;
            Ok(items
                .iter()
                .zip(rows_by_item)
                .map(|(item, rows)| view_result(item, ViewOutcome::PeerRows(rows)))
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
                outcome: ViewOutcome::View(view),
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

fn view_result(item: &BatchItem, outcome: ViewOutcome) -> MetricViewResult {
    MetricViewResult {
        metric_index: item.metric_index,
        view_index: item.view_index,
        outcome,
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
        .with_setting("log_comment", log_comment);
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
