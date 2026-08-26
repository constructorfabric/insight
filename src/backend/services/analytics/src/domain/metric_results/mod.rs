mod batch;
mod builder;
pub(crate) mod compiler;
mod dto;
mod failure;
#[cfg(test)]
mod live_tests;
mod validation;
mod view;

pub use batch::{
    BatchItem, PeerPopulation, PeerWideRow, PeriodWideRow, PlannedQuery, RankingResults,
    UnbatchedView, demux_peer_rows, demux_period_rows, plan_queries, plan_rankings,
};
pub use builder::{
    build_breakdown_view, build_histogram_view, build_metric_result, build_peer_view,
    build_period_view, build_pooled_histogram_view, build_ranked_groups, build_rollup_view,
    build_timeseries_view, enforce_view_row_limit,
};
pub use compiler::{
    BreakdownQueryRow, CompiledQuery, HistogramQueryRow, PooledHistogramQueryRow, RankingQueryRow,
    RollupQueryRow, TimeseriesQueryRow,
};
pub use dto::{
    MetricDimensionFilterDto, MetricResultSelectionDto, MetricResultViewDto,
    MetricResultsEntityDto, MetricResultsPeriodDto, MetricResultsRequest, MetricResultsResponse,
};
pub use failure::ViewFailure;
pub use validation::{ValidatedMetricResultsRequest, validate_request};
pub(crate) use validation::{normalize_key, normalize_metric_key};
