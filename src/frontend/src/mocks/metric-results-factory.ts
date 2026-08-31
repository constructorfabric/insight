import type {
  MetricErrorView,
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

export function metaFor(metricKey: string): Omit<MetricResult, "views"> {
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

/**
 * Values a dimension breaks down into, so a mock answers the dimension it was
 * ASKED for. Returning one fixed set meant a section reading any other
 * dimension found nothing in its rows and rendered as absent — which reads as
 * a broken screen rather than as a mock with no fixture.
 */
const MOCK_DIMENSION_VALUES: Record<
  string,
  { value: string; label: string }[]
> = {
  category: [
    { value: "code", label: "Code" },
    { value: "test", label: "Tests" },
    { value: "docs", label: "Documentation" },
    { value: "config", label: "Configuration" },
    { value: "vendored", label: "Vendored / Generated" },
  ],
  branch_scope: [
    { value: "default", label: "Default branch" },
    { value: "non_default", label: "Other branches" },
  ],
  source: [
    { value: "github", label: "GitHub" },
    { value: "gitlab", label: "GitLab" },
  ],
};

function mockDimensionValues(key: string): { value: string; label: string }[] {
  return (
    MOCK_DIMENSION_VALUES[key] ??
    MOCK_TOOLS.map(({ value, label }) => ({ value, label }))
  );
}

/**
 * A failed-computation view, delivered in a requested view's slot the way the
 * backend does since ClickHouse error isolation: the request still answers
 * 200 and the other views/metrics are unaffected.
 */
export function buildMetricErrorView(
  overrides: Partial<Omit<MetricErrorView, "view">> = {},
): MetricErrorView {
  return {
    view: "error",
    code: "QUERY_FAILED",
    message:
      "This metric could not be computed; the rest of the results are unaffected. An administrator can see the underlying error.",
    ...overrides,
  };
}

export function buildMetricResultsResponse(
  request: MetricResultsRequest,
): MetricResultsResponse {
  const ids =
    request.entity.type === "person"
      ? request.entity.ids
      : ["00000000-0000-4000-8000-00000000c0de"];
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
        case "breakdown": {
          // One row per combination the caller asked for, keyed by the
          // requested dimension rather than by a fixed one. Every key draws
          // from its OWN vocabulary: a composition splitting repositories by
          // branch scope would otherwise label its segments with repository
          // names, which reads as a broken screen rather than as a mock.
          const axis = view.dimensions[0] ?? "tool";
          return {
            view: "breakdown",
            dimensions: view.dimensions,
            values: ids.flatMap((entityId) =>
              mockDimensionValues(axis).map((dimension, index) => ({
                entity_id: entityId,
                dimensions: view.dimensions.map((dimensionKey, keyIndex) => {
                  const values = mockDimensionValues(dimensionKey);
                  const pick = values[(index + keyIndex) % values.length] ?? dimension;
                  return {
                    key: dimensionKey,
                    value: pick.value,
                    label: pick.label,
                  };
                }),
                value: valueFor(entityId, key, dimension.value),
              })),
            ),
          };
        }
        case "rollup":
          return {
            view: "rollup",
            dimensions: view.dimensions,
            values: MOCK_TOOLS.map((dimension) => ({
              dimensions: view.dimensions.map((key) => ({
                key,
                value: dimension.value,
                label: dimension.label,
              })),
              value: valueFor("rollup", key, dimension.value),
              contributing_entity_count: ids.length,
            })),
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
    // Every mock metric carries evidence and its canonical selection: without
    // both, no surface offers the drilldown at all and mock mode cannot reach
    // the records dialog.
    return {
      ...meta,
      views,
      drilldown: { granularity: ["event"] },
      selection: {
        metric_key: key,
        entity: request.entity,
        period: request.period,
        filters: metricRequest.filters ?? [],
      },
    } as MetricResult;
  });

  return { metrics };
}
