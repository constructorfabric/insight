/**
 * Wire-level fixtures for the `<MetricTimeseriesView>` stories.
 *
 * The view owns its own data fetching, so a story drives it through MSW rather
 * than props. The handler answers `POST /metric-results` from a small fact cube
 * (repository × source × bucket), honouring the parts of the request the view
 * actually varies — dimension filters, group-by dimension, and `group_limit`
 * (top-N + remainder) — so group/filter interactions change the rendered
 * numbers instead of replaying one canned payload.
 */

import { delay, http, HttpResponse } from "msw";

import type {
  MetricDimensionFilter,
  MetricRequest,
  MetricResult,
  MetricResultView,
  MetricResultsRequest,
  MetricResultsResponse,
  MetricViewRequest,
} from "@/api/metric-results-client";

export const ENTITY_ID = "alice@example.com";
export const RANGE = { from: "2026-04-20", to: "2026-05-04" };
export const BUCKETS = ["2026-04-20", "2026-04-27", "2026-05-04"];

export const COMMITS = "git.commits";
export const LINES_ADDED = "git.lines_added";

const METRIC_META = {
  [COMMITS]: { label: "Commits", unit: "commits" },
  [LINES_ADDED]: { label: "Lines added", unit: "lines" },
} as const;

type MetricKey = keyof typeof METRIC_META;

const METRIC_KEYS = Object.keys(METRIC_META) as MetricKey[];

interface Fact {
  dimensions: { repository: string; source: string };
  points: Record<MetricKey, Array<number | null>>;
}

type DimensionKey = keyof Fact["dimensions"];

const FACTS: Fact[] = [
  {
    dimensions: { repository: "org/repo-a", source: "github" },
    points: { [COMMITS]: [3, 0, 2], [LINES_ADDED]: [30, 0, 24] },
  },
  {
    dimensions: { repository: "org/repo-b", source: "github" },
    points: { [COMMITS]: [2, null, 1], [LINES_ADDED]: [18, null, 9] },
  },
  {
    dimensions: { repository: "org/repo-c", source: "gitlab" },
    points: { [COMMITS]: [1, 1, 0], [LINES_ADDED]: [11, 6, 0] },
  },
];

const DIMENSION_LABELS: Record<string, Record<string, string>> = {
  source: { github: "GitHub", gitlab: "GitLab" },
};

interface Group {
  value: string;
  label: string;
  points: Record<MetricKey, Array<number | null>>;
}

function emptyPoints(): Record<MetricKey, Array<number | null>> {
  return Object.fromEntries(
    METRIC_KEYS.map((key) => [key, BUCKETS.map(() => null)])
  ) as Record<MetricKey, Array<number | null>>;
}

function addPoints(
  target: Array<number | null>,
  source: Array<number | null>
): void {
  source.forEach((value, index) => {
    if (value == null) return;
    target[index] = (target[index] ?? 0) + value;
  });
}

function totalOf(points: Array<number | null>): number | null {
  return points.reduce<number | null>(
    (total, value) => (value == null ? total : (total ?? 0) + value),
    null
  );
}

function labelFor(dimension: string | undefined, value: string): string {
  if (!dimension) return "Total";
  return DIMENSION_LABELS[dimension]?.[value] ?? value;
}

function matchesFilters(
  fact: Fact,
  filters: readonly MetricDimensionFilter[]
): boolean {
  return filters.every((filter) =>
    filter.values.includes(fact.dimensions[filter.dimension as DimensionKey])
  );
}

/** Facts aggregated by one dimension (or into a single group when unset). */
function groupFacts(
  dimension: string | undefined,
  filters: readonly MetricDimensionFilter[]
): Group[] {
  const groups = new Map<string, Group>();
  for (const fact of FACTS) {
    if (!matchesFilters(fact, filters)) continue;
    const value = dimension ? fact.dimensions[dimension as DimensionKey] : "";
    const group = groups.get(value) ?? {
      value,
      label: labelFor(dimension, value),
      points: emptyPoints(),
    };
    for (const key of METRIC_KEYS)
      addPoints(group.points[key], fact.points[key]);
    groups.set(value, group);
  }
  return [...groups.values()];
}

function rankGroups(groups: Group[], rankMetricKey: MetricKey): Group[] {
  return [...groups].sort(
    (left, right) =>
      (totalOf(right.points[rankMetricKey]) ?? 0) -
        (totalOf(left.points[rankMetricKey]) ?? 0) ||
      left.label.localeCompare(right.label)
  );
}

function mergeGroups(groups: Group[], label: string): Group {
  const merged: Group = { value: label, label, points: emptyPoints() };
  for (const group of groups) {
    for (const key of METRIC_KEYS)
      addPoints(merged.points[key], group.points[key]);
  }
  return merged;
}

function isMetricKey(key: string): key is MetricKey {
  return key in METRIC_META;
}

function timeseriesView(
  metricKey: MetricKey,
  view: Extract<MetricViewRequest, { view: "timeseries" }>,
  filters: readonly MetricDimensionFilter[]
): MetricResultView {
  const dimension = view.dimensions?.[0];
  const limit = view.group_limit;
  const rankBy = limit?.rank_by_metric;
  const ranked = rankGroups(
    groupFacts(dimension, filters),
    rankBy && isMetricKey(rankBy) ? rankBy : metricKey
  );
  const kept = limit ? ranked.slice(0, limit.count) : ranked;
  const dropped = limit ? ranked.slice(limit.count) : [];
  const remainder =
    limit?.include_remainder && dropped.length > 0
      ? mergeGroups(dropped, "Other")
      : null;

  const pointsOf = (group: Group) =>
    BUCKETS.map((bucket_start, index) => ({
      bucket_start,
      value: group.points[metricKey][index] ?? null,
    }));

  return {
    view: "timeseries",
    bucket: view.bucket ?? "day",
    series: [
      ...kept.map((group, index) => ({
        entity_id: ENTITY_ID,
        dimensions: dimension
          ? [{ key: dimension, value: group.value, label: group.label }]
          : [],
        total: totalOf(group.points[metricKey]),
        ...(dimension ? { rank: index + 1 } : {}),
        points: pointsOf(group),
      })),
      ...(remainder
        ? [
            {
              entity_id: ENTITY_ID,
              dimensions: [],
              remainder: true,
              label: remainder.label,
              total: totalOf(remainder.points[metricKey]),
              points: pointsOf(remainder),
            },
          ]
        : []),
    ],
  };
}

function resultViews(
  metricKey: MetricKey,
  request: MetricRequest
): MetricResultView[] {
  const filters = request.filters ?? [];
  return request.views.map((view) => {
    switch (view.view) {
      case "period":
        return {
          view: "period",
          values: [
            {
              entity_id: ENTITY_ID,
              value: totalOf(
                groupFacts(undefined, filters)[0]?.points[metricKey] ?? []
              ),
            },
          ],
        };
      case "timeseries":
        return timeseriesView(metricKey, view, filters);
      case "breakdown": {
        const dimension = view.dimensions[0];
        return {
          view: "breakdown",
          dimensions: view.dimensions,
          values: groupFacts(dimension, filters).map((group) => ({
            entity_id: ENTITY_ID,
            dimensions: dimension
              ? [{ key: dimension, value: group.value, label: group.label }]
              : [],
            value: totalOf(group.points[metricKey]),
          })),
        };
      }
      default:
        throw new Error(`unsupported view in story fixtures: ${view.view}`);
    }
  });
}

export function buildStoryMetricResults(
  request: MetricResultsRequest
): MetricResultsResponse {
  const metrics = request.metrics.flatMap<MetricResult>((metricRequest) => {
    const key = metricRequest.metric_key;
    if (!isMetricKey(key)) return [];
    return [
      {
        metric_key: key,
        label: METRIC_META[key].label,
        unit: METRIC_META[key].unit,
        format: "integer",
        direction: "higher_is_better",
        computation: "sum",
        views: resultViews(key, metricRequest),
      },
    ];
  });
  return { metrics };
}

export type MetricResultsScenario =
  /** Answers from the fact cube; `delayMs` keeps a refetch visibly in flight. */
  | { kind: "data"; delayMs?: number }
  /** No metrics — the view's empty state. */
  | { kind: "empty" }
  /** Never resolves — the view's initial loading state. */
  | { kind: "pending" }
  /** Always fails — the view's error state. */
  | { kind: "error" }
  /** Fails `state.failures` times, then serves data — exercises Retry. */
  | { kind: "recovering"; state: { failures: number } };

const METRIC_RESULTS_URL = "/api/analytics/v1/metric-results";

export function metricResultsHandler(
  scenario: MetricResultsScenario = { kind: "data" }
) {
  return http.post(METRIC_RESULTS_URL, async ({ request }) => {
    switch (scenario.kind) {
      case "pending":
        await delay("infinite");
        break;
      case "error":
        return new HttpResponse(null, { status: 503 });
      case "empty":
        return HttpResponse.json({ metrics: [] });
      case "recovering":
        if (scenario.state.failures > 0) {
          scenario.state.failures -= 1;
          return new HttpResponse(null, { status: 503 });
        }
        break;
      case "data":
        if (scenario.delayMs) await delay(scenario.delayMs);
        break;
    }
    const body = (await request.json()) as MetricResultsRequest;
    return HttpResponse.json(buildStoryMetricResults(body));
  });
}
