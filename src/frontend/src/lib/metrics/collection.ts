import type { DateRange } from "@/api/period-to-date-range";
import type {
  BreakdownView,
  HistogramView,
  MetricBucket,
  MetricComputation,
  MetricDirection,
  MetricDimensionFilter,
  MetricEntityType,
  MetricFormat,
  MetricGroupLimit,
  MetricResult,
  MetricResultViewKind,
  MetricResultsRequest,
  MetricViewRequest,
  PeerView,
  PeriodView,
  TimeseriesView,
} from "@/api/metric-results-client";

type MetricCollectionBucket = MetricBucket | "auto";

export interface MetricTimeseriesGroupLimitConfig {
  count: number;
  rankBy?: string;
  includeRemainder: boolean;
}

type MetricCollectionViewConfig =
  | { view: "period" }
  | { view: "peer" }
  | {
      view: "timeseries";
      bucket: MetricCollectionBucket;
      dimensions?: string[];
      groupLimit?: MetricGroupLimit;
    }
  | {
      view: "breakdown";
      dimensions: string[];
    }
  | { view: "histogram" };

interface MetricCollectionMetricConfig {
  key: string;
  filters?: MetricDimensionFilter[];
  views: MetricCollectionViewConfig[];
}

export interface MetricCollectionConfig {
  metrics: MetricCollectionMetricConfig[];
}

export interface MetricCollectionEntity {
  type: MetricEntityType;
  ids: string[];
}

export type NormalizedMetricResult = {
  metric_key: string;
  label: string;
  /** Compact label for dense surfaces (member grids); falls back to `label`. */
  short_label?: string;
  description?: string;
  explanation?: string;
  unit: string | null;
  computation: MetricComputation;
  format: MetricFormat;
  direction: MetricDirection;
  scale?: number;
  period?: PeriodView;
  timeseries?: TimeseriesView;
  peer?: PeerView;
  breakdown?: BreakdownView;
  histogram?: HistogramView;
  drilldown?: MetricResult["drilldown"];
  selection?: MetricResult["selection"];
};

export type PeerEntityStats = PeerView["values"][number];

export interface EntityMetricData {
  value: number | null;
  peer: PeerEntityStats | null;
  bucket: MetricBucket | null;
  series: TimeseriesView["series"];
  breakdown: BreakdownView["values"];
  histogram: HistogramView["values"];
}

function exhaustive(value: never): never {
  throw new Error(
    `Unhandled metric collection variant: ${JSON.stringify(value)}`
  );
}

export function buildMetricCollectionRequest(
  collection: MetricCollectionConfig,
  entity: MetricCollectionEntity,
  period: DateRange
): MetricResultsRequest {
  return {
    entity: { type: entity.type, ids: entity.ids },
    period,
    metrics: collection.metrics.map((metric) => ({
      metric_key: metric.key,
      ...(metric.filters?.length ? { filters: metric.filters } : {}),
      views: metric.views.map((view) => toRequestView(view, period)),
    })),
  };
}

/**
 * Derive a view-subset collection for a surface that must not request the
 * full view set (e.g. team surfaces request `period` + `peer` only — a
 * per-member timeseries over a large roster would exceed the backend's
 * all-or-nothing row limit and fail the whole request).
 */
export function projectViews(
  collection: MetricCollectionConfig,
  kinds: readonly MetricResultViewKind[]
): MetricCollectionConfig {
  return {
    metrics: collection.metrics
      .map((metric) => ({
        key: metric.key,
        filters: metric.filters,
        views: metric.views.filter((view) => kinds.includes(view.view)),
      }))
      // The backend rejects a metric with no views (invalid_argument) and
      // fails the whole request — drop rather than send empty.
      .filter((metric) => metric.views.length > 0),
  };
}

export function normalizeMetricResult(
  metric: MetricResult
): NormalizedMetricResult {
  const normalized: NormalizedMetricResult = {
    metric_key: metric.metric_key,
    label: metric.label,
    short_label: metric.short_label,
    description: metric.description,
    explanation: metric.explanation,
    unit: metric.unit,
    computation: metric.computation,
    format: metric.format,
    direction: metric.direction,
    scale: metric.computation === "ratio" ? metric.scale : undefined,
  };
  if (metric.drilldown) normalized.drilldown = metric.drilldown;
  if (metric.selection) normalized.selection = metric.selection;

  for (const view of metric.views) {
    switch (view.view) {
      case "period":
        normalized.period = view;
        break;
      case "timeseries":
        normalized.timeseries = view;
        break;
      case "peer":
        normalized.peer = view;
        break;
      case "breakdown":
        normalized.breakdown = view;
        break;
      case "histogram":
        normalized.histogram = view;
        break;
      default:
        // Forward-compat: the server may ship new view kinds before this
        // client knows them; an unknown view must not take down the whole
        // collection. `exhaustive` stays on request-side unions only.
        if (import.meta.env.DEV) {
          console.warn("Ignoring unknown metric view kind", view);
        }
        break;
    }
  }

  return normalized;
}

export function normalizeMetricResults(
  metrics: MetricResult[] | undefined
): Map<string, NormalizedMetricResult> {
  return new Map(
    (metrics ?? []).map((metric) => {
      const normalized = normalizeMetricResult(metric);
      return [normalized.metric_key, normalized];
    })
  );
}

/**
 * Entity-scoped read of a normalized result. Renderers and cell selectors are
 * entity-blind: they receive the slice for one entity instead of re-running
 * `values.find(v => v.entity_id === …)` each.
 */
export function forEntity(
  result: NormalizedMetricResult,
  entityId: string
): EntityMetricData {
  return {
    value:
      result.period?.values.find((v) => v.entity_id === entityId)?.value ??
      null,
    peer: result.peer?.values.find((v) => v.entity_id === entityId) ?? null,
    bucket: result.timeseries?.bucket ?? null,
    series:
      result.timeseries?.series.filter((s) => s.entity_id === entityId) ?? [],
    breakdown:
      result.breakdown?.values.filter((v) => v.entity_id === entityId) ?? [],
    histogram:
      result.histogram?.values.filter((v) => v.entity_id === entityId) ?? [],
  };
}

/**
 * The backend rejects requests whose projected result rows exceed its
 * all-or-nothing limit (5000). Headroom below that so period+peer chunking
 * never sits exactly on the cliff.
 */
const MAX_PROJECTED_ROWS = 4500;

/**
 * How many entities fit in one request for this collection, or null when the
 * collection is not chunkable. Timeseries/breakdown project rows per bucket
 * or dimension group (unbounded client-side); histogram projects a fixed bin
 * count per entity but is a drilldown-only single-entity view — all three are
 * for single-entity surfaces and roster surfaces strip them via
 * `projectViews(["period", "peer"])`, so none should ride a chunked request.
 */
export function entityChunkSize(
  collection: MetricCollectionConfig
): number | null {
  let rowsPerEntity = 0;
  for (const metric of collection.metrics) {
    for (const view of metric.views) {
      if (
        view.view === "timeseries" ||
        view.view === "breakdown" ||
        view.view === "histogram"
      ) {
        return null;
      }
      rowsPerEntity += 1;
    }
  }
  if (rowsPerEntity === 0) return null;
  return Math.max(1, Math.floor(MAX_PROJECTED_ROWS / rowsPerEntity));
}

export function chunkEntityIds(ids: string[], size: number): string[][] {
  if (ids.length <= size) return [ids];
  const chunks: string[][] = [];
  for (let i = 0; i < ids.length; i += size) {
    chunks.push(ids.slice(i, i + size));
  }
  return chunks;
}

/**
 * Merge per-chunk results for the same collection: entity-scoped view values
 * concatenate; metric metadata is identical across chunks (first wins). Only
 * period/peer views occur here — chunking is disabled for collections with
 * timeseries/breakdown views.
 */
export function mergeNormalizedResults(
  maps: Array<Map<string, NormalizedMetricResult>>
): Map<string, NormalizedMetricResult> {
  const out = new Map<string, NormalizedMetricResult>();
  for (const map of maps) {
    for (const [key, result] of map) {
      const existing = out.get(key);
      if (!existing) {
        out.set(key, {
          ...result,
          period: result.period
            ? { ...result.period, values: [...result.period.values] }
            : undefined,
          peer: result.peer
            ? { ...result.peer, values: [...result.peer.values] }
            : undefined,
        });
        continue;
      }
      if (existing.period && result.period) {
        existing.period.values.push(...result.period.values);
      } else if (result.period) {
        existing.period = {
          ...result.period,
          values: [...result.period.values],
        };
      }
      if (existing.peer && result.peer) {
        existing.peer.values.push(...result.peer.values);
      } else if (result.peer) {
        existing.peer = { ...result.peer, values: [...result.peer.values] };
      }
    }
  }
  return out;
}

function toRequestView(
  view: MetricCollectionViewConfig,
  period: DateRange
): MetricViewRequest {
  switch (view.view) {
    case "period":
      return { view: "period" };
    case "peer":
      return { view: "peer" };
    case "timeseries":
      return {
        view: "timeseries",
        bucket: resolveBucket(view.bucket, period),
        dimensions: view.dimensions,
        group_limit: view.groupLimit,
      };
    case "breakdown":
      return { view: "breakdown", dimensions: view.dimensions };
    case "histogram":
      return { view: "histogram" };
    default:
      exhaustive(view);
  }
}

const MAX_DAY_BUCKET_DAYS = 7;
const MIN_MONTH_BUCKET_DAYS = 365;

export function resolveBucket(
  bucket: MetricCollectionBucket,
  period: DateRange
): MetricBucket {
  if (bucket !== "auto") return bucket;
  const days = daysInRange(period);
  if (days <= MAX_DAY_BUCKET_DAYS) return "day";
  if (days >= MIN_MONTH_BUCKET_DAYS) return "month";
  return "week";
}

export function resolveTimeseriesBucket(period: DateRange): MetricBucket {
  return resolveBucket("auto", period);
}

function daysInRange(period: DateRange): number {
  const from = parseDateUtcMs(period.from);
  const to = parseDateUtcMs(period.to);
  if (from == null || to == null || from > to) return 0;
  return Math.floor((to - from) / 86_400_000) + 1;
}

function parseDateUtcMs(value: string): number | null {
  const parts = value.split("-").map(Number);
  if (parts.length !== 3) return null;
  const [year, month, day] = parts;
  if (!year || !month || !day) return null;
  const timestamp = Date.UTC(year, month - 1, day);
  const date = new Date(timestamp);
  return date.getUTCFullYear() === year &&
    date.getUTCMonth() === month - 1 &&
    date.getUTCDate() === day
    ? timestamp
    : null;
}
