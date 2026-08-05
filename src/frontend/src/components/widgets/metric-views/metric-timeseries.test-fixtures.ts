import type { SumMetricResult } from "@/api/metric-results-client";
import { buildMetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";
import { normalizeMetricResults } from "@/lib/metrics/collection";

export const ENTITY_ID = "alice@example.com";
export const RANGE = { from: "2026-04-20", to: "2026-05-04" };

const commits: SumMetricResult = {
  metric_key: "git.commits",
  label: "Commits",
  unit: "commits",
  format: "integer",
  direction: "higher_is_better",
  computation: "sum",
  views: [
    {
      view: "period",
      values: [{ entity_id: ENTITY_ID, value: 6 }],
    },
    {
      view: "timeseries",
      bucket: "week",
      series: [
        {
          entity_id: ENTITY_ID,
          dimensions: [
            {
              key: "repository",
              value: "org/repo-b",
              label: "org/repo-b",
            },
          ],
          total: 3,
          rank: 2,
          points: [
            { bucket_start: "2026-04-20", value: 2 },
            { bucket_start: "2026-04-27", value: null },
            { bucket_start: "2026-05-04", value: 1 },
          ],
        },
        {
          entity_id: ENTITY_ID,
          dimensions: [
            {
              key: "repository",
              value: "org/repo-a",
              label: "org/repo-a",
            },
          ],
          total: 3,
          rank: 1,
          points: [
            { bucket_start: "2026-04-20", value: 3 },
            { bucket_start: "2026-04-27", value: 0 },
          ],
        },
      ],
    },
    {
      view: "breakdown",
      dimensions: ["repository"],
      values: [
        {
          entity_id: ENTITY_ID,
          dimensions: [
            {
              key: "repository",
              value: "org/repo-b",
              label: "org/repo-b",
            },
          ],
          value: 3,
        },
        {
          entity_id: ENTITY_ID,
          dimensions: [
            {
              key: "repository",
              value: "org/repo-a",
              label: "org/repo-a",
            },
          ],
          value: 3,
        },
      ],
    },
  ],
};

const lines: SumMetricResult = {
  ...commits,
  metric_key: "git.lines_added",
  label: "Lines added",
  unit: "lines",
  views: commits.views.map((view) => {
    if (view.view === "period") {
      return {
        ...view,
        values: [{ entity_id: ENTITY_ID, value: 120 }],
      };
    }
    if (view.view === "timeseries") {
      return {
        ...view,
        series: view.series.map((series, index) => ({
          ...series,
          total: index === 0 ? 90 : 30,
          points: series.points.map((point) => ({
            ...point,
            value: point.value == null ? null : point.value * (index + 10),
          })),
        })),
      };
    }
    if (view.view === "breakdown") {
      return {
        ...view,
        values: view.values.map((value, index) => ({
          ...value,
          value: index === 0 ? 90 : 30,
        })),
      };
    }
    return view;
  }),
};

export function timeseriesByKey() {
  return normalizeMetricResults([commits, lines]);
}

export function groupedTimeseriesModel() {
  return buildMetricTimeseriesModel(
    timeseriesByKey(),
    ["git.commits", "git.lines_added"],
    ENTITY_ID,
    RANGE,
    ["repository"]
  );
}
