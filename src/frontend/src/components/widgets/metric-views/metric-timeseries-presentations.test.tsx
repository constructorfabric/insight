import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { MetricTimeseriesChart } from "@/components/widgets/metric-views/metric-timeseries-chart";
import {
  buildMetricTimeseriesChartModel,
  commonNullRuns,
} from "@/components/widgets/metric-views/metric-timeseries-chart-model";
import { MetricTimeseriesTable } from "@/components/widgets/metric-views/metric-timeseries-table";
import { resolveMetricTimeseriesTableColumns } from "@/components/widgets/metric-views/metric-timeseries-table-model";
import { groupedTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries.test-fixtures";
import type { MetricTimeseriesTableConfig } from "@/lib/metrics/timeseries-table";

const COMPOSED_TABLE = {
  columns: [
    { metric: "git.commits" },
    {
      label: "Activity",
      template: [
        {
          metric: "git.lines_added",
          prefix: "+",
          tone: "success",
        },
        { text: " / " },
        {
          metric: "git.commits",
          prefix: "−",
          tone: "destructive",
        },
      ],
    },
  ],
} satisfies MetricTimeseriesTableConfig;

describe("metric timeseries presentations", () => {
  it("renders grouped metrics in a multi-level table", () => {
    render(<MetricTimeseriesTable model={groupedTimeseriesModel()} />);
    expect(screen.getByText("Week")).toBeInTheDocument();
    expect(screen.getAllByText("org/repo-a").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Commits").length).toBeGreaterThan(0);
    expect(screen.getByText("Grand total").closest("tr")).toHaveTextContent(
      "Commits: 6"
    );
    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  });

  it("renders a grouped single-metric table with a single header row", () => {
    const grouped = groupedTimeseriesModel();
    const model = {
      ...grouped,
      metrics: [grouped.metrics[0]!],
      grandTotals: [grouped.grandTotals[0]],
    };
    render(<MetricTimeseriesTable model={model} />);
    // One header cell per dimension column, no per-metric subheader row.
    expect(screen.getByText("org/repo-a")).toBeInTheDocument();
    expect(screen.getByText("org/repo-b")).toBeInTheDocument();
    expect(screen.queryByText("Commits")).not.toBeInTheDocument();
    expect(screen.getByText("Grand total")).toBeInTheDocument();
  });

  it("hides the grand-total row when every total is missing", () => {
    const grouped = groupedTimeseriesModel();
    const model = {
      ...grouped,
      grandTotals: grouped.grandTotals.map(() => null),
    };
    render(<MetricTimeseriesTable model={model} />);
    expect(screen.queryByText("Grand total")).not.toBeInTheDocument();
  });

  it("renders an ungrouped single-metric table", () => {
    const grouped = groupedTimeseriesModel();
    const model = {
      ...grouped,
      dimensions: [],
      metrics: [grouped.metrics[0]!],
      columns: [grouped.columns[0]!],
      grandTotals: [grouped.grandTotals[0]],
    };
    render(<MetricTimeseriesTable model={model} />);
    expect(screen.getByText("Commits")).toBeInTheDocument();
    expect(screen.queryByText("Grand total")).not.toBeInTheDocument();
  });

  it("renders configured metric templates as one table column", () => {
    render(
      <MetricTimeseriesTable
        model={groupedTimeseriesModel()}
        config={COMPOSED_TABLE}
      />
    );
    expect(screen.getAllByText("Activity")).toHaveLength(2);
    expect(screen.queryByText("Lines added")).not.toBeInTheDocument();
    expect(screen.getAllByText("+33")[0]).toHaveClass("text-success");
    expect(screen.getAllByText("−3")[0]).toHaveClass("text-destructive");
    expect(screen.getByText(/Activity:/)).toBeInTheDocument();
  });

  it("uses a configured short metric label", () => {
    const model = groupedTimeseriesModel();
    const configuredModel = {
      ...model,
      metrics: model.metrics.map((metric) =>
        metric.metric_key === "git.commits"
          ? { ...metric, short_label: "Changes" }
          : metric
      ),
    };
    const columns = resolveMetricTimeseriesTableColumns(configuredModel, {
      columns: [{ metric: "git.commits", labelSource: "short" }],
    });
    expect(columns).toHaveLength(1);
    expect(columns[0]?.label).toBe("Changes");
  });

  it("falls back to the normal metric label when the short label is absent", () => {
    const columns = resolveMetricTimeseriesTableColumns(
      groupedTimeseriesModel(),
      {
        columns: [{ metric: "git.commits", labelSource: "short" }],
      }
    );
    expect(columns).toHaveLength(1);
    expect(columns[0]?.label).toBe("Commits");
  });

  it("distinguishes missing values from observed zeroes in templates", () => {
    const model = groupedTimeseriesModel();
    const firstColumn = model.columns[0]!;
    const linePoints = new Map(firstColumn.points.get("git.lines_added"));
    linePoints.set("2026-04-20", null);
    const points = new Map(firstColumn.points);
    points.set("git.lines_added", linePoints);
    const configuredModel = {
      ...model,
      columns: [{ ...firstColumn, points }, ...model.columns.slice(1)],
    };
    render(
      <MetricTimeseriesTable model={configuredModel} config={COMPOSED_TABLE} />
    );
    const rows = screen.getAllByRole("row");
    expect(within(rows[2]!).getAllByRole("cell")[2]).toHaveTextContent(
      "— / −3"
    );
    expect(within(rows[3]!).getAllByRole("cell")[2]).toHaveTextContent(
      "+0 / −0"
    );
  });

  it("renders grouped and ungrouped chart variants", () => {
    const grouped = groupedTimeseriesModel();
    const { rerender } = render(
      <MetricTimeseriesChart model={grouped} selectedMetricKey="git.commits" />
    );
    expect(screen.getByText("org/repo-a")).toBeInTheDocument();
    expect(screen.getAllByText(/3 commits/)).toHaveLength(2);
    rerender(
      <MetricTimeseriesChart
        model={{
          ...grouped,
          dimensions: [],
          metrics: [grouped.metrics[0]!],
          columns: [grouped.columns[0]!],
        }}
        selectedMetricKey="missing"
      />
    );
    expect(screen.queryByText("org/repo-a")).not.toBeInTheDocument();
  });

  it("renders multiple metrics together when configured", () => {
    const grouped = groupedTimeseriesModel();
    const model = {
      ...grouped,
      dimensions: [],
      columns: [
        {
          ...grouped.columns[0]!,
          key: "total",
          colorSeed: "total",
          label: "Total",
        },
      ],
    };
    render(
      <MetricTimeseriesChart
        model={model}
        selectedMetricKey="git.commits"
        multiMetric="combined"
      />
    );
    expect(screen.getByText("Commits")).toBeInTheDocument();
    expect(screen.getByText("Lines added")).toBeInTheDocument();
  });

  it("projects each combined metric into its own series", () => {
    const grouped = groupedTimeseriesModel();
    const sourceColumn = grouped.columns[0]!;
    const chartModel = buildMetricTimeseriesChartModel(
      {
        ...grouped,
        dimensions: [],
        columns: [sourceColumn],
      },
      "git.commits",
      "combined"
    );

    expect(chartModel?.series.map((series) => series.label)).toEqual([
      "Commits",
      "Lines added",
    ]);
    expect(chartModel?.series[0]?.points).toBe(
      sourceColumn.points.get("git.commits")
    );
    expect(chartModel?.series[1]?.points).toBe(
      sourceColumn.points.get("git.lines_added")
    );
  });

  it("finds only contiguous buckets missing from every displayed series", () => {
    const buckets = ["a", "b", "c", "d", "e"];
    const series = [
      new Map<string, number | null>([
        ["a", 1],
        ["b", null],
        ["c", null],
        ["d", 2],
        ["e", null],
      ]),
      new Map<string, number | null>([
        ["a", 1],
        ["b", null],
        ["c", null],
        ["d", null],
        ["e", null],
      ]),
    ];

    expect(commonNullRuns(buckets, series)).toEqual([
      { startIndex: 1, endIndex: 2 },
      { startIndex: 4, endIndex: 4 },
    ]);
  });

  it("labels multi-bucket gaps without connecting the line", () => {
    const grouped = groupedTimeseriesModel();
    const metric = grouped.metrics[0]!;
    const sourceColumn = grouped.columns[0]!;
    const points = new Map(sourceColumn.points);
    points.set(
      metric.metric_key,
      new Map([
        ["2026-04-20", 3],
        ["2026-04-27", null],
        ["2026-05-04", null],
      ])
    );

    render(
      <MetricTimeseriesChart
        model={{
          ...grouped,
          dimensions: [],
          metrics: [metric],
          columns: [{ ...sourceColumn, points }],
        }}
        selectedMetricKey={metric.metric_key}
      />
    );

    expect(
      document.querySelector(".recharts-reference-area-rect")
    ).toBeInTheDocument();
    expect(screen.getByText("No data")).toBeInTheDocument();
  });
});
