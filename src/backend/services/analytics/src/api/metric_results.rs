use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::Extension;
use axum::http::HeaderMap;
use futures::stream::{self, StreamExt};
use serde::de::DeserializeOwned;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::AppState;
use crate::config::VisibilityPolicy;
use crate::domain::metric_access::authorize_tenant_metrics;
use crate::domain::metric_definitions::MetricDefinition;
use crate::domain::metric_drilldown::load_capabilities;
use crate::domain::metric_results::{
    BatchItem, BreakdownQueryRow, CompiledQuery, HistogramQueryRow, MetricResultViewDto,
    MetricResultsRequest, MetricResultsResponse, PeerPopulation, PeerWideRow, PeriodWideRow,
    PlannedQuery, PooledHistogramQueryRow, RankingQueryRow, RankingResults, RollupQueryRow,
    TimeseriesQueryRow, UnbatchedView, ValidatedMetricResultsRequest, ViewFailure,
    build_breakdown_view, build_histogram_view, build_metric_result, build_peer_view,
    build_period_view, build_pooled_histogram_view, build_ranked_groups, build_rollup_view,
    build_timeseries_view, demux_peer_rows, demux_period_rows, enforce_view_row_limit,
    plan_queries, plan_rankings, validate_request,
};
use crate::domain::person_visibility::authorize_person_ids;

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
    authorize_tenant_request(&state, &req)?;
    let mut req = validate_request(&state.db, tenant_id, req).await?;
    req.enforce_tenant_scope = state.config.metric_catalog.enforce_tenant_scope;
    authorize_person_request(&state, &ctx, &headers, &req).await?;

    let metric_keys = req
        .metrics
        .iter()
        .map(|metric| metric.def.key().to_owned())
        .collect::<Vec<_>>();
    let capabilities = load_capabilities(&state.db, tenant_id, &metric_keys);
    let (ranking_results, capabilities) =
        tokio::join!(collect_rankings(&state, &req), capabilities);
    let peer_population = peer_population(state.config.visibility_policy);
    let planned = plan_queries(&req, &ranking_results, peer_population)?;

    let mut views_by_metric: Vec<Vec<Option<Result<MetricResultViewDto, ViewFailure>>>> = req
        .metrics
        .iter()
        .map(|metric| (0..metric.views.len()).map(|_| None).collect())
        .collect();

    // A failed query settles only its own view slots as errors; the other
    // queries keep running so one broken metric cannot empty the response.
    let mut results = stream::iter(planned)
        .map(|query| execute_planned(&state, &req, query))
        .buffer_unordered(QUERY_CONCURRENCY);
    while let Some(result) = results.next().await {
        for view in result {
            views_by_metric[view.metric_index][view.view_index] = Some(view.view);
        }
    }

    let failed_views = views_by_metric
        .iter()
        .flatten()
        .filter(|view| matches!(view, Some(Err(_))))
        .count();
    let admin = failed_views > 0 && admin_for_error_detail(&state, &headers).await;
    if failed_views > 0 {
        tracing::warn!(
            failed_views,
            "metric-results answered with per-view failures"
        );
    }

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
        for (view_index, view) in views_by_metric[idx].drain(..).enumerate() {
            let Some(view) = view else {
                return Err(CanonicalError::internal("missing metric view result").create());
            };
            let view = match view {
                Ok(view) => {
                    enforce_view_row_limit(&view, format!("metrics[{idx}].views[{view_index}]"))?;
                    view
                }
                Err(failure) => failure.into_view(admin),
            };
            views.push(view);
        }
        let selection = crate::domain::metric_results::MetricResultSelectionDto {
            metric_key: metric.def.key().to_owned(),
            entity: if req.entity.is_tenant() {
                crate::domain::metric_results::MetricResultsEntityDto::Tenant {}
            } else {
                crate::domain::metric_results::MetricResultsEntityDto::Person {
                    ids: req.entity.entity_ids(),
                }
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

fn peer_population(visibility_policy: VisibilityPolicy) -> PeerPopulation {
    match visibility_policy {
        VisibilityPolicy::OrgChart => PeerPopulation::DeclaredCohort,
        VisibilityPolicy::Flat => PeerPopulation::Tenant,
    }
}

fn authorize_tenant_request(
    state: &AppState,
    req: &MetricResultsRequest,
) -> Result<(), CanonicalError> {
    if req.entity.is_tenant() {
        authorize_tenant_metrics(state.config.metric_catalog.tenant_metrics_enabled)?;
    }

    Ok(())
}

async fn authorize_person_request(
    state: &AppState,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    req: &ValidatedMetricResultsRequest,
) -> Result<(), CanonicalError> {
    let Some(person_ids) = req.entity.person_ids() else {
        return Ok(());
    };

    authorize_person_ids(
        &state.identity,
        ctx,
        super::forwarded_authorization(headers),
        person_ids,
    )
    .await
}

struct MetricViewResult {
    metric_index: usize,
    view_index: usize,
    view: Result<MetricResultViewDto, ViewFailure>,
}

// A ranking that fails settles as a failure keyed by its policy, so only the
// views that asked for that ranking answer with an error.
async fn collect_rankings(
    state: &Arc<AppState>,
    req: &ValidatedMetricResultsRequest,
) -> RankingResults {
    let mut ranking_results = RankingResults::default();
    let mut rankings = stream::iter(plan_rankings(req))
        .map(|ranking| {
            let state = Arc::clone(state);
            async move {
                let comment = format!("metric-results:ranking:{}", ranking.key.rank_metric_key);
                let outcome =
                    match fetch_rows::<RankingQueryRow>(&state, ranking.query, &comment).await {
                        Ok(rows) => build_ranked_groups(&ranking.dimensions, rows)
                            .map_err(|e| assembly_failure(&e, &comment)),
                        Err(failure) => Err(failure),
                    };
                (ranking.key, outcome)
            }
        })
        .buffer_unordered(QUERY_CONCURRENCY);

    while let Some((key, outcome)) = rankings.next().await {
        match outcome {
            Ok(groups) => {
                ranking_results.groups.insert(key, groups);
            }
            Err(failure) => {
                ranking_results.failures.insert(key, failure);
            }
        }
    }
    ranking_results
}

async fn execute_planned(
    state: &Arc<AppState>,
    req: &ValidatedMetricResultsRequest,
    planned: PlannedQuery,
) -> Vec<MetricViewResult> {
    match planned {
        PlannedQuery::PeriodBatch { items, query } => {
            let comment = batch_log_comment("period", &items);
            let outcome = match fetch_rows::<PeriodWideRow>(state, query, &comment).await {
                Ok(rows) => {
                    demux_period_rows(&items, rows).map_err(|e| assembly_failure(&e, &comment))
                }
                Err(failure) => Err(failure),
            };
            match outcome {
                Ok(rows_by_item) => items
                    .iter()
                    .zip(rows_by_item)
                    .map(|(item, rows)| {
                        view_result(item, Ok(build_period_view(&item.def, req, rows)))
                    })
                    .collect(),
                Err(failure) => fail_batch(&items, &failure),
            }
        }
        PlannedQuery::PeerBatch { items, query } => {
            let comment = batch_log_comment("peer", &items);
            let outcome = match fetch_rows::<PeerWideRow>(state, query, &comment).await {
                Ok(rows) => {
                    demux_peer_rows(&items, rows).map_err(|e| assembly_failure(&e, &comment))
                }
                Err(failure) => Err(failure),
            };
            match outcome {
                Ok(rows_by_item) => items
                    .iter()
                    .zip(rows_by_item)
                    .map(|(item, rows)| view_result(item, Ok(build_peer_view(rows))))
                    .collect(),
                Err(failure) => fail_batch(&items, &failure),
            }
        }
        PlannedQuery::Single {
            metric_index,
            view_index,
            def,
            view,
            query,
        } => {
            let view = build_single_view(state, req, &def, view, query).await;
            vec![MetricViewResult {
                metric_index,
                view_index,
                view,
            }]
        }
        PlannedQuery::Failed {
            metric_index,
            view_index,
            failure,
        } => vec![MetricViewResult {
            metric_index,
            view_index,
            view: Err(failure),
        }],
    }
}

async fn build_single_view(
    state: &Arc<AppState>,
    req: &ValidatedMetricResultsRequest,
    def: &MetricDefinition,
    view: UnbatchedView,
    query: CompiledQuery,
) -> Result<MetricResultViewDto, ViewFailure> {
    match view {
        UnbatchedView::Timeseries {
            bucket, dimensions, ..
        } => {
            let comment = format!("metric-results:timeseries:{}", def.key());
            let rows = fetch_rows::<TimeseriesQueryRow>(state, query, &comment).await?;
            build_timeseries_view(def, req, bucket, &dimensions, rows)
                .map_err(|e| assembly_failure(&e, &comment))
        }
        UnbatchedView::Breakdown { dimensions } => {
            let comment = format!("metric-results:breakdown:{}", def.key());
            let rows = fetch_rows::<BreakdownQueryRow>(state, query, &comment).await?;
            build_breakdown_view(req, &dimensions, rows, &state.external_links)
                .map_err(|e| assembly_failure(&e, &comment))
        }
        UnbatchedView::Rollup { dimensions } => {
            let comment = format!("metric-results:rollup:{}", def.key());
            let rows = fetch_rows::<RollupQueryRow>(state, query, &comment).await?;
            build_rollup_view(&dimensions, rows).map_err(|e| assembly_failure(&e, &comment))
        }
        UnbatchedView::Histogram => {
            let comment = format!("metric-results:histogram:{}", def.key());
            let rows = fetch_rows::<HistogramQueryRow>(state, query, &comment).await?;
            Ok(build_histogram_view(req, rows))
        }
        UnbatchedView::PooledHistogram { dimensions } => {
            let comment = format!("metric-results:pooled-histogram:{}", def.key());
            let rows = fetch_rows::<PooledHistogramQueryRow>(state, query, &comment).await?;
            build_pooled_histogram_view(rows, &dimensions)
                .map_err(|e| assembly_failure(&e, &comment))
        }
    }
}

fn fail_batch(items: &[BatchItem], failure: &ViewFailure) -> Vec<MetricViewResult> {
    items
        .iter()
        .map(|item| view_result(item, Err(failure.clone())))
        .collect()
}

fn assembly_failure(error: &CanonicalError, comment: &str) -> ViewFailure {
    tracing::error!(error = ?error, comment, "metric view assembly failed");
    ViewFailure::from_assembly_error(comment)
}

// Detail routing only: identity being unreachable must not turn a partial
// answer into a 500, so an unanswerable admin check degrades to the generic
// message rather than failing the request.
async fn admin_for_error_detail(state: &AppState, headers: &HeaderMap) -> bool {
    match super::is_admin_caller(state, headers).await {
        Ok(admin) => admin,
        Err(error) => {
            tracing::warn!(error = ?error, "admin check for error detail failed; using generic messages");
            false
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

fn view_result(
    item: &BatchItem,
    view: Result<MetricResultViewDto, ViewFailure>,
) -> MetricViewResult {
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
) -> Result<Vec<T>, ViewFailure>
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
        tracing::error!(error = %e, comment = log_comment, sql = %query.sql, "ClickHouse metric-results query failed");
        ViewFailure::from_query_error(&e.to_string())
    })?;

    let raw_bytes = tokio::time::timeout(QUERY_FETCH_TIMEOUT, cursor.collect())
        .await
        .map_err(|_| {
            tracing::error!(comment = log_comment, sql = %query.sql, "ClickHouse metric-results fetch timed out");
            ViewFailure::timeout()
        })?
        .map_err(|e| {
            tracing::error!(error = %e, comment = log_comment, sql = %query.sql, "ClickHouse metric-results fetch failed");
            ViewFailure::from_query_error(&e.to_string())
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
            tracing::error!(error = %e, comment = log_comment, sql = %query.sql, "failed to parse metric-results rows");
            ViewFailure::from_parse_error(&e.to_string())
        })
}

#[cfg(test)]
mod tests {
    use crate::config::VisibilityPolicy;
    use crate::domain::metric_results::PeerPopulation;

    use super::peer_population;

    #[test]
    fn visibility_policy_selects_peer_population() {
        assert_eq!(
            peer_population(VisibilityPolicy::OrgChart),
            PeerPopulation::DeclaredCohort
        );
        assert_eq!(
            peer_population(VisibilityPolicy::Flat),
            PeerPopulation::Tenant
        );
    }
}
