import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

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

/**
 * jsdom has no layout engine and no ResizeObserver, so every dimension reads
 * zero and the table concludes that nothing overflows. Supplying the geometry
 * is what lets the real decisions run — which side holds more, how far a page
 * moves — rather than a stand-in for them.
 */
afterEach(() => vi.unstubAllGlobals());

/** Scoped to the tests that need it: charts here measure themselves and a
 *  no-op observer leaves them with nothing to draw. */
function stubResizeObserver(): void {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    }
  );
}

function giveGeometry(
  container: HTMLElement,
  box: Partial<
    Record<
      | "clientWidth"
      | "scrollWidth"
      | "clientHeight"
      | "scrollHeight"
      | "scrollLeft"
      | "scrollTop",
      number
    >
  >
): HTMLElement & { scrollBy: ReturnType<typeof vi.fn> } {
  const el = container.querySelector<HTMLElement>(
    '[data-slot="table-container"]'
  )!;
  for (const [key, value] of Object.entries(box)) {
    Object.defineProperty(el, key, { value, configurable: true });
  }
  const scrollBy = vi.fn();
  Object.defineProperty(el, "scrollBy", { value: scrollBy, configurable: true });
  fireEvent.scroll(el);
  return el as HTMLElement & { scrollBy: typeof scrollBy };
}

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

  it("offers no paging control towards a side with nothing on it", () => {
    // Nothing overflows here: jsdom reports every dimension as zero.
    render(<MetricTimeseriesTable model={groupedTimeseriesModel()} />);
    expect(
      screen.queryByRole("button", { name: /Show (earlier|later)/ })
    ).toBeNull();
  });

  it("offers a control for each side that still holds something", () => {
    stubResizeObserver();
    const { container } = render(
      <MetricTimeseriesTable model={groupedTimeseriesModel()} />
    );
    giveGeometry(container, {
      clientWidth: 400,
      scrollWidth: 1200,
      clientHeight: 200,
      scrollHeight: 600,
      scrollLeft: 0,
      scrollTop: 0,
    });
    // Sitting at both starts: only the two forward controls have anywhere to go.
    expect(screen.getByRole("button", { name: "Show later columns" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Show later rows" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Show earlier columns" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Show earlier rows" })).toBeNull();
  });

  it("pages by just under a frame, away from the edge it points at", () => {
    stubResizeObserver();
    const { container } = render(
      <MetricTimeseriesTable model={groupedTimeseriesModel()} />
    );
    const box = giveGeometry(container, {
      clientWidth: 400,
      scrollWidth: 1200,
      clientHeight: 200,
      scrollHeight: 600,
      scrollLeft: 500,
      scrollTop: 300,
    });

    fireEvent.click(screen.getByRole("button", { name: "Show later columns" }));
    expect(box.scrollBy).toHaveBeenLastCalledWith({
      left: 320,
      behavior: "smooth",
    });

    fireEvent.click(screen.getByRole("button", { name: "Show earlier columns" }));
    expect(box.scrollBy).toHaveBeenLastCalledWith({
      left: -320,
      behavior: "smooth",
    });

    fireEvent.click(screen.getByRole("button", { name: "Show later rows" }));
    expect(box.scrollBy).toHaveBeenLastCalledWith({
      top: 160,
      behavior: "smooth",
    });

    fireEvent.click(screen.getByRole("button", { name: "Show earlier rows" }));
    expect(box.scrollBy).toHaveBeenLastCalledWith({
      top: -160,
      behavior: "smooth",
    });
  });

  it("pins the totals to the foot of the box, with an edge over the rows", () => {
    render(<MetricTimeseriesTable model={groupedTimeseriesModel()} />);
    const foot = screen.getByText("Grand total").closest("tfoot")!;
    expect(foot.className).toContain("sticky");
    expect(foot.className).toContain("bottom-0");
    expect(foot.className).toContain("inset_0_1px_0_0");
    // Opaque, not tinted: the row the footer covers showed through sliced
    // along its middle, and the totals read as riding on it.
    expect(foot.className).toMatch(/(^|\s)bg-muted(\s|$)/);
  });

  it("keeps the grand total beside its label when the table is scrolled", () => {
    // The cell spans every group, so its own start edge scrolls away.
    render(<MetricTimeseriesTable model={groupedTimeseriesModel()} />);
    const row = screen.getByText("Grand total").closest("tr")!;
    const totals = row.querySelectorAll("td")[1]!.querySelector("span")!;
    expect(totals.className).toContain("sticky");
    expect(totals.className).toContain("start-24");
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
