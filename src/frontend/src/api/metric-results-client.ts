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
  | "histogram";
export type MetricBucket = "day" | "week" | "month";
export type MetricComputation = "sum" | "ratio" | "median" | "distinct_count";
export type MetricEntityType = "person";

export interface MetricResultsRequest {
  entity: { type: MetricEntityType; ids: string[] };
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
  entity: { type: MetricEntityType; ids: string[] };
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
  | { view: "histogram" };

export interface MetricDimension {
  key: string;
  value: string;
  label?: string;
}

export type MetricResult =
  | SumMetricResult
  | RatioMetricResult
  | MedianMetricResult
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

interface DistinctCountMetricResult extends MetricResultBase {
  computation: "distinct_count";
}

export type MetricResultView =
  | PeriodView
  | TimeseriesView
  | PeerView
  | BreakdownView
  | HistogramView;

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

export interface MetricResultsResponse {
  metrics: MetricResult[];
}

export async function queryMetricResults(
  body: MetricResultsRequest
): Promise<MetricResultsResponse> {
  const res = await fetchWithAuth(`${BASE}/metric-results`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
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
