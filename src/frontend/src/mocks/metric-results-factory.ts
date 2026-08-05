import type {
  MetricResult,
  MetricResultView,
  MetricResultsRequest,
  MetricResultsResponse,
} from "@/api/metric-results-client";

import { metricResultFixtureByKey } from "./metric-results-fixtures";

/**
 * Builds a `/v1/metric-results` response from the request body: every
 * requested metric and view is echoed back (the backend never returns
 * partial views), with deterministic per-(entity, metric) values so the UI
 * is stable across reloads. Metric metadata comes from the wire fixtures
 * when the key is known, synthesized otherwise.
 */

function hash(input: string): number {
  let h = 2166136261;
  for (let i = 0; i < input.length; i += 1) {
    h ^= input.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h);
}

function valueFor(entityId: string, metricKey: string, salt = ""): number {
  return (hash(`${entityId}|${metricKey}|${salt}`) % 900) + 50;
}

function metaFor(metricKey: string): Omit<MetricResult, "views"> {
  const fixture = metricResultFixtureByKey(metricKey);
  if (fixture) {
    const { views: _views, ...meta } = fixture;
    return meta;
  }
  const label = metricKey
    .split(".")
    .pop()!
    .replaceAll("_", " ")
    .replace(/^./, (c) => c.toUpperCase());
  return {
    metric_key: metricKey,
    label,
    unit: null,
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
  };
}

function bucketStarts(from: string, count: number): string[] {
  const [y, m, d] = from.split("-").map(Number);
  const out: string[] = [];
  for (let i = 0; i < count; i += 1) {
    const date = new Date(Date.UTC(y ?? 2026, (m ?? 1) - 1, (d ?? 1) + i));
    out.push(date.toISOString().slice(0, 10));
  }
  return out;
}

const MOCK_TOOLS = [
  { key: "tool", value: "claude_code", label: "Claude Code" },
  { key: "tool", value: "cursor", label: "Cursor" },
];

export function buildMetricResultsResponse(
  request: MetricResultsRequest,
): MetricResultsResponse {
  const ids = request.entity.ids;
  const metrics: MetricResult[] = request.metrics.map((metricRequest) => {
    const meta = metaFor(metricRequest.metric_key);
    const key = metricRequest.metric_key;
    const views: MetricResultView[] = metricRequest.views.map((view) => {
      switch (view.view) {
        case "period":
          return {
            view: "period",
            values: ids.map((entityId) => ({
              entity_id: entityId,
              value: valueFor(entityId, key),
            })),
          };
        case "peer":
          return {
            view: "peer",
            values: ids.map((entityId) => {
              const median = valueFor("cohort-median", key);
              return {
                entity_id: entityId,
                target_value: valueFor(entityId, key),
                p25: median * 0.6,
                median,
                p75: median * 1.4,
                min: median * 0.2,
                max: median * 2,
                n: 12,
              };
            }),
          };
        case "timeseries": {
          const starts = bucketStarts(request.period.from, 7);
          return {
            view: "timeseries",
            bucket: view.bucket ?? "day",
            series: ids.flatMap((entityId) =>
              (view.dimensions?.length ? MOCK_TOOLS : [null]).map(
                (dimension) => ({
                  entity_id: entityId,
                  dimensions: dimension ? [dimension] : [],
                  points: starts.map((bucket_start, index) => ({
                    bucket_start,
                    value:
                      valueFor(
                        entityId,
                        key,
                        `${bucket_start}|${dimension?.value ?? "total"}`,
                      ) %
                      (30 + index),
                  })),
                }),
              ),
            ),
          };
        }
        case "breakdown":
          return {
            view: "breakdown",
            dimensions: view.dimensions,
            values: ids.flatMap((entityId) =>
              MOCK_TOOLS.map((dimension) => ({
                entity_id: entityId,
                dimensions: [dimension],
                value: valueFor(entityId, key, dimension.value),
              })),
            ),
          };
        case "histogram":
          return {
            view: "histogram",
            values: ids.map((entityId) => {
              const width = 10;
              return {
                entity_id: entityId,
                bins: Array.from({ length: 6 }, (_, i) => ({
                  lo: i * width,
                  hi: (i + 1) * width,
                  count: valueFor(entityId, key, `bin${i}`) % 40,
                })),
              };
            }),
          };
        default:
          return view satisfies never;
      }
    });
    return { ...meta, views } as MetricResult;
  });

  return { metrics };
}
