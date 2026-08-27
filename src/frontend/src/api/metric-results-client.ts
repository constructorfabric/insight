import { fetchWithAuth } from "@/api/fetch-with-auth";
import { AnalyticsApiError } from "@/api/analytics-client";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export type MetricFormat = "integer" | "decimal" | "currency" | "percent";
export type MetricDirection =
  | "higher_is_better"
  | "lower_is_better"
  | "neutral";
export type MetricResultViewKind =
  | "period"
  | "timeseries"
  | "peer"
  | "breakdown"
  | "rollup"
  | "histogram";
export type MetricBucket = "day" | "week" | "month";
export type MetricComputation =
  | "sum"
  | "ratio"
  | "median"
  | "percentile"
  | "stddev"
  | "distinct_count";
export type MetricEntityType = "person" | "tenant";
export type MetricResultsEntity =
  | { type: "person"; ids: string[] }
  | { type: "tenant" };

export interface MetricResultsRequest {
  entity: MetricResultsEntity;
  period: { from: string; to: string };
  metrics: MetricRequest[];
}

export interface MetricRequest {
  metric_key: string;
  filters?: MetricDimensionFilter[];
  views: MetricViewRequest[];
}

export interface MetricDimensionFilter {
  dimension: string;
  values: string[];
}

export interface MetricDrilldownCapability {
  granularity: Array<"event" | "source_summary" | "derived_population">;
}

export interface MetricCanonicalSelection {
  metric_key: string;
  entity: MetricResultsEntity;
  period: { from: string; to: string };
  filters: MetricDimensionFilter[];
}

export interface MetricGroupLimit {
  count: number;
  rank_by_metric?: string;
  include_remainder: boolean;
}

export type MetricViewRequest =
  | { view: "period" }
  | { view: "peer"; cohort_key?: string }
  | {
      view: "timeseries";
      bucket?: MetricBucket;
      dimensions?: string[];
      group_limit?: MetricGroupLimit;
    }
  | {
      view: "breakdown";
      dimensions: string[];
    }
  | {
      view: "rollup";
      dimensions: string[];
      group_limit?: MetricGroupLimit;
    }
  | { view: "histogram" };

export interface MetricDimension {
  key: string;
  value: string;
  label?: string;
  href?: string;
}

export type MetricResult =
  | SumMetricResult
  | RatioMetricResult
  | MedianMetricResult
  | PercentileMetricResult
  | StddevMetricResult
  | DistinctCountMetricResult;

interface MetricResultBase {
  metric_key: string;
  label: string;
  /** Compact label for dense surfaces (member grids); falls back to `label`. */
  short_label?: string;
  description?: string;
  explanation?: string;
  unit: string | null;
  format: MetricFormat;
  direction: MetricDirection;
  views: MetricResultView[];
  drilldown?: MetricDrilldownCapability;
  selection?: MetricCanonicalSelection;
}

export interface SumMetricResult extends MetricResultBase {
  computation: "sum";
}

export interface RatioMetricResult extends MetricResultBase {
  computation: "ratio";
  scale: number;
}

export interface MedianMetricResult extends MetricResultBase {
  computation: "median";
}

export interface PercentileMetricResult extends MetricResultBase {
  computation: "percentile";
  q: number;
}

export interface StddevMetricResult extends MetricResultBase {
  computation: "stddev";
}

interface DistinctCountMetricResult extends MetricResultBase {
  computation: "distinct_count";
}

export type MetricResultView =
  | PeriodView
  | TimeseriesView
  | PeerView
  | BreakdownView
  | RollupView
  | HistogramView
  | MetricErrorView;

export interface PeriodView {
  view: "period";
  values: Array<{ entity_id: string; value: number | null }>;
}

export interface TimeseriesView {
  view: "timeseries";
  bucket: MetricBucket;
  series: Array<{
    entity_id: string;
    dimensions: MetricDimension[];
    total?: number | null;
    rank?: number;
    remainder?: boolean;
    label?: string;
    points: Array<{ bucket_start: string; value: number | null }>;
  }>;
}

export interface PeerView {
  view: "peer";
  values: Array<{
    entity_id: string;
    target_value: number | null;
    p25: number | null;
    median: number | null;
    p75: number | null;
    min: number | null;
    max: number | null;
    n: number;
  }>;
}

export interface BreakdownView {
  view: "breakdown";
  dimensions: string[];
  values: Array<{
    entity_id: string;
    dimensions: MetricDimension[];
    value: number | null;
  }>;
}

export interface RollupView {
  view: "rollup";
  dimensions: string[];
  values: Array<{
    dimensions: MetricDimension[];
    value: number | null;
    contributing_entity_count: number;
    rank?: number;
    remainder?: boolean;
    label?: string;
  }>;
}

interface HistogramBin {
  lo: number;
  hi: number;
  count: number;
}

export interface HistogramView {
  view: "histogram";
  values: Array<{
    entity_id: string;
    bins: HistogramBin[];
  }>;
}

export type MetricErrorCode =
  | "SOURCE_RELATION_MISSING"
  | "RESOURCE_EXHAUSTED"
  | "QUERY_TIMEOUT"
  | "RESULT_PARSE_FAILED"
  | "QUERY_FAILED";

/**
 * A view whose computation failed. The request itself still answers 200:
 * the failure arrives in the failed view's requested slot, and other views
 * and metrics are unaffected. `message` is safe to render — admins get the
 * underlying error text, everyone else a generic one.
 */
export interface MetricErrorView {
  view: "error";
  code: MetricErrorCode;
  message: string;
}

export interface MetricResultsResponse {
  metrics: MetricResult[];
}

export async function queryMetricResults(
  body: MetricResultsRequest,
  signal?: AbortSignal
): Promise<MetricResultsResponse> {
  // Refuse a request the backend is guaranteed to reject (400 invalid_argument,
  // "entity.ids must not be empty"). Callers are expected to keep the query
  // disabled until they have entities; failing here names the real cause
  // instead of surfacing a server validation error in the network log.
  if (body.entity.type === "person" && body.entity.ids.length === 0) {
    throw new Error(
      "metric-results: refusing to request an empty entity list — the caller should stay disabled until the roster resolves",
    );
  }
  const res = await fetchWithAuth(`${BASE}/metric-results`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  if (!res.ok) {
    const errorBody = await res.json().catch(() => null);
    throw new AnalyticsApiError(res.status, errorBody);
  }
  try {
    return (await res.json()) as MetricResultsResponse;
  } catch {
    throw new AnalyticsApiError(res.status, { error: "invalid_json" });
  }
}
