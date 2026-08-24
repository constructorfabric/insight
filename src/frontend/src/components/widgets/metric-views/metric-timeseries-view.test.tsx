const usageMocks = vi.hoisted(() => ({ recordUsageEvent: vi.fn() }));

import { useEffect } from "react";

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { EvidenceDialogContext } from "@/components/metric-evidence-context";
import { MetricTimeseriesView } from "@/components/widgets/metric-views/metric-timeseries-view";
import {
  ENTITY_ID,
  RANGE,
  groupedTimeseriesModel,
  timeseriesByKey,
} from "@/components/widgets/metric-views/metric-timeseries.test-fixtures";

const mocks = vi.hoisted(() => ({
  collection: vi.fn(),
  collectionSet: vi.fn(),
  csv: vi.fn(),
  xlsx: vi.fn(),
  evidenceColumn: "total",
  tableOverflows: false,
}));

vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: mocks.collection,
  useMetricCollectionSet: mocks.collectionSet,
}));

vi.mock("@/components/widgets/metric-views/metric-timeseries-chart", () => ({
  MetricTimeseriesChart: ({
    onEvidence,
  }: {
    onEvidence?: (
      metricKey: string,
      columnKey: string,
      bucketStart: string | null
    ) => void;
  }) => (
    <div>
      chart presentation
      <button
        type="button"
        onClick={() =>
          onEvidence?.("git.commits", mocks.evidenceColumn, "2026-04-20")
        }
      >
        drill point
      </button>
    </div>
  ),
}));

vi.mock("@/components/widgets/metric-views/metric-timeseries-table", () => ({
  MetricTimeseriesTable: ({
    onVerticalOverflow,
  }: {
    onVerticalOverflow?: (overflows: boolean) => void;
  }) => {
    // The real table measures and reports; jsdom cannot, so the test says.
    useEffect(() => {
      onVerticalOverflow?.(mocks.tableOverflows);
    }, [onVerticalOverflow]);
    return <div>table presentation</div>;
  },
}));

vi.mock("@/telemetry", async () => {
  const actual = await vi.importActual<typeof import("@/telemetry")>("@/telemetry");
  return { ...actual, recordUsageEvent: usageMocks.recordUsageEvent };
});

vi.mock("@/components/widgets/metric-views/metric-timeseries-csv", () => ({
  downloadMetricTimeseriesCsv: mocks.csv,
}));

vi.mock("@/components/widgets/metric-views/metric-timeseries-xlsx", () => ({
  downloadMetricTimeseriesXlsx: mocks.xlsx,
}));

const ready = {
  byKey: timeseriesByKey(),
  previousByKey: null,
  isPending: false,
  isFetching: false,
  isError: false,
  refetch: vi.fn(),
};

describe("MetricTimeseriesView", () => {
  beforeEach(() => {
    localStorage.clear();
    mocks.collection.mockReturnValue(ready);
    mocks.collectionSet.mockReturnValue(new Map());
    mocks.csv.mockReset();
    mocks.xlsx.mockReset().mockResolvedValue(undefined);
    mocks.tableOverflows = false;
  });

  it("switches presentations and persists presentation per card", async () => {
    const user = userEvent.setup();
    render(
      <MetricTimeseriesView
        id="git-output"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits", "git.lines_added"]}
        groupBy={{ default: "repository" }}
      />
    );
    expect(screen.getByText("chart presentation")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Collapse card" })
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Table view" }));
    expect(screen.getByText("table presentation")).toBeInTheDocument();
    expect(
      localStorage.getItem("insight.timeseries.git-output.presentation")
    ).toBe("table");
  });

  it("renders pending, error, and empty states", () => {
    mocks.collection.mockReturnValue({ ...ready, isPending: true });
    const { container, rerender } = render(
      <MetricTimeseriesView
        id="states"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits"]}
      />
    );
    expect(container.querySelector("[aria-busy] svg")).toBeInTheDocument();
    mocks.collection.mockReturnValue({ ...ready, isError: true });
    rerender(
      <MetricTimeseriesView
        id="states"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits"]}
      />
    );
    expect(screen.getByText("Unable to load timeseries")).toBeInTheDocument();
    mocks.collection.mockReturnValue({ ...ready, byKey: new Map() });
    rerender(
      <MetricTimeseriesView
        id="states"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits"]}
      />
    );
    expect(screen.getByText("No data in this period")).toBeInTheDocument();
  });

  it("builds bounded grouped requests without a totals breakdown", () => {
    render(
      <MetricTimeseriesView
        id="request"
        entityId={ENTITY_ID}
        range={{ from: "2026-04-20", to: "2026-04-20" }}
        metricKeys={["git.commits"]}
        groupBy={{
          default: "repository",
          options: ["source"],
          limits: {
            repository: {
              count: 10,
              rankBy: "git.commits",
              includeRemainder: true,
            },
          },
        }}
      />
    );
    expect(mocks.collection.mock.calls.at(-1)?.[0]).toMatchObject({
      metrics: [
        {
          key: "git.commits",
          views: [
            {
              view: "timeseries",
              bucket: "day",
              dimensions: ["repository"],
              groupLimit: {
                count: 10,
                rank_by_metric: "git.commits",
                include_remainder: true,
              },
            },
            { view: "period" },
          ],
        },
      ],
    });
    expect(mocks.collectionSet.mock.calls.at(-1)?.[0]).toMatchObject([
      {
        key: "source",
        collection: {
          metrics: [
            {
              key: "git.commits",
              views: [{ view: "breakdown", dimensions: ["source"] }],
            },
          ],
        },
      },
    ]);
  });

  it("does not cap an uncapped active dimension", () => {
    render(
      <MetricTimeseriesView
        id="uncapped"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.lines_added"]}
        groupBy={{
          default: "category",
          options: ["repository"],
          limits: {
            repository: {
              count: 10,
              rankBy: "git.lines_added",
              includeRemainder: true,
            },
          },
        }}
      />
    );
    expect(mocks.collection.mock.calls.at(-1)?.[0]).toMatchObject({
      metrics: [
        {
          views: [
            {
              view: "timeseries",
              dimensions: ["category"],
            },
            { view: "period" },
          ],
        },
      ],
    });
    expect(
      mocks.collection.mock.calls.at(-1)?.[0].metrics[0].views[0]
    ).not.toHaveProperty("groupLimit");
  });

  it("exports Excel and CSV through the export menu", async () => {
    const user = userEvent.setup();
    render(
      <MetricTimeseriesView
        id="exp"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits"]}
      />
    );

    await user.click(screen.getByRole("button", { name: "Export" }));
    await user.click(await screen.findByText("Excel (.xlsx)"));
    expect(mocks.xlsx).toHaveBeenCalledTimes(1);
    expect(mocks.xlsx.mock.calls[0]?.[0]).toBe("exp");
    expect(mocks.xlsx.mock.calls[0]?.[2]).toEqual(RANGE);

    await user.click(screen.getByRole("button", { name: "Export" }));
    await user.click(await screen.findByText("CSV (.csv)"));
    expect(mocks.csv).toHaveBeenCalledTimes(1);
    expect(mocks.csv.mock.calls[0]?.[0]).toBe("exp");
  });

  it("reports which format a reader took the data out in", async () => {
    const user = userEvent.setup();
    usageMocks.recordUsageEvent.mockClear();
    render(
      <MetricTimeseriesView
        id="exp"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits"]}
      />
    );

    await user.click(screen.getByRole("button", { name: "Export" }));
    await user.click(await screen.findByText("Excel (.xlsx)"));
    expect(usageMocks.recordUsageEvent).toHaveBeenCalledWith(
      "export",
      "timeseries:xlsx",
    );

    await user.click(screen.getByRole("button", { name: "Export" }));
    await user.click(await screen.findByText("CSV (.csv)"));
    expect(usageMocks.recordUsageEvent).toHaveBeenCalledWith(
      "export",
      "timeseries:csv",
    );
  });

  it("switches the charted metric through the metric select", async () => {
    const user = userEvent.setup();
    render(
      <MetricTimeseriesView
        id="pick"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits", "git.lines_added"]}
      />
    );

    const trigger = screen.getByLabelText("Metric");
    expect(trigger).toHaveTextContent("Commits");

    await user.click(trigger);
    await user.click(
      await screen.findByRole("option", { name: "Lines added" })
    );

    expect(screen.getByLabelText("Metric")).toHaveTextContent("Lines added");
  });

  it("shows a combined title instead of a metric selector", () => {
    render(
      <MetricTimeseriesView
        id="combined"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits", "git.lines_added"]}
        chart={{ multiMetric: "combined" }}
      />
    );

    expect(screen.queryByLabelText("Metric")).not.toBeInTheDocument();
    expect(screen.getByText("Commits & Lines added")).toBeInTheDocument();
  });

  it("lets the reader drop the height ceiling, and puts it back on the way out", async () => {
    mocks.tableOverflows = true;
    const user = userEvent.setup();
    const { container } = render(
      <MetricTimeseriesView
        id="expandable"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits", "git.lines_added"]}
        groupBy={{ default: "repository" }}
      />
    );
    const body = () => container.querySelector('[data-slot="card-content"]')!;
    await user.click(screen.getByRole("button", { name: "Table view" }));
    expect(body().className).toContain("max-h-96");

    await user.click(screen.getByRole("button", { name: "Show every row" }));
    expect(body().className).not.toContain("max-h-96");
    expect(
      screen.getByRole("button", { name: "Scroll the table" })
    ).toHaveAttribute("aria-pressed", "true");

    // Round trip through the chart: the ceiling comes back rather than the
    // next table opening uncapped.
    await user.click(screen.getByRole("button", { name: "Chart view" }));
    await user.click(screen.getByRole("button", { name: "Table view" }));
    expect(body().className).toContain("max-h-96");
  });

  it("lets a table size itself while holding a chart to a fixed box", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <MetricTimeseriesView
        id="sized"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits", "git.lines_added"]}
        groupBy={{ default: "repository" }}
      />
    );
    const body = () => container.querySelector('[data-slot="card-content"]')!;
    expect(body().className).toContain("h-96");
    expect(body().className).not.toContain("max-h-96");

    await user.click(screen.getByRole("button", { name: "Table view" }));
    expect(body().className).toContain("max-h-96");
  });

  it("names a grouped table by its grouping, with how many groups there are", async () => {
    const user = userEvent.setup();
    render(
      <MetricTimeseriesView
        id="by-repo"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits", "git.lines_added"]}
        groupBy={{ default: "repository" }}
      />
    );
    await user.click(screen.getByRole("button", { name: "Table view" }));
    const heading = screen.getByRole("heading", { name: /By repository/ });
    expect(heading).toHaveTextContent("By repository");
    expect(heading).toHaveTextContent("2");
    expect(screen.queryByLabelText("Metric")).not.toBeInTheDocument();
  });

  it("uses visible group controls and supports selecting multiple filters", async () => {
    const user = userEvent.setup();
    const options = timeseriesByKey();
    const metric = options.get("git.commits");
    if (!metric) throw new Error("missing fixture metric");
    metric.breakdown = {
      view: "breakdown",
      dimensions: ["source"],
      values: [
        {
          entity_id: ENTITY_ID,
          dimensions: [{ key: "source", value: "github", label: "GitHub" }],
          value: 4,
        },
        {
          entity_id: ENTITY_ID,
          dimensions: [{ key: "source", value: "gitlab", label: "GitLab" }],
          value: 2,
        },
      ],
    };
    mocks.collectionSet.mockReturnValue(
      new Map([
        [
          "source",
          {
            byKey: options,
            isPending: false,
            isFetching: false,
            isError: false,
          },
        ],
      ])
    );
    render(
      <MetricTimeseriesView
        id="controls"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits"]}
        groupBy={{ default: "repository", options: ["source"] }}
      />
    );

    await user.click(screen.getByRole("button", { name: "Filters" }));
    await user.click(screen.getByRole("checkbox", { name: "GitHub" }));
    await user.click(screen.getByRole("checkbox", { name: "GitLab" }));
    expect(mocks.collection.mock.calls.at(-1)?.[0].metrics[0].filters).toEqual([
      { dimension: "source", values: ["github", "gitlab"] },
    ]);

    await user.click(screen.getByRole("button", { name: "Source" }));
    expect(
      mocks.collection.mock.calls.at(-1)?.[0].metrics[0].views[0]
    ).toMatchObject({
      view: "timeseries",
      dimensions: ["source"],
    });
  });

  it("overlays a spinner while revalidating already-shown data", () => {
    mocks.collection.mockReturnValue({ ...ready, isFetching: true });
    const { container } = render(
      <MetricTimeseriesView
        id="reval"
        entityId={ENTITY_ID}
        range={RANGE}
        metricKeys={["git.commits"]}
      />
    );

    expect(container.querySelector('[aria-busy="true"]')).toBeInTheDocument();
    expect(screen.getByText("chart presentation")).toBeInTheDocument();
    // The export action is disabled while fetching.
    expect(screen.getByRole("button", { name: "Export" })).toBeDisabled();
  });

  it("opens all targets for a combined chart", async () => {
    const user = userEvent.setup();
    const byKey = timeseriesByKey();
    for (const metric of byKey.values()) {
      metric.drilldown = { granularity: ["event"] };
      metric.unit = "commits";
      metric.selection = {
        metric_key: metric.metric_key,
        entity: { type: "person", ids: [ENTITY_ID] },
        period: RANGE,
        filters: [],
      };
    }
    mocks.collection.mockReturnValue({ ...ready, byKey });
    mocks.evidenceColumn = "total";
    const openEvidence = vi.fn();
    const openEvidenceTargets = vi.fn();
    const openEvidencePeople = vi.fn();
    render(
      <EvidenceDialogContext.Provider
        value={{ openEvidence, openEvidenceTargets, openEvidencePeople }}
      >
        <MetricTimeseriesView
          id="evidence"
          entityId={ENTITY_ID}
          range={RANGE}
          metricKeys={["git.commits", "git.lines_added"]}
          chart={{ multiMetric: "combined" }}
        />
      </EvidenceDialogContext.Provider>
    );

    await user.click(
      screen.getByRole("button", { name: "View supporting data" })
    );
    expect(openEvidenceTargets).toHaveBeenCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          selection: expect.objectContaining({
            metric_key: "git.commits",
            display_dimensions: [],
          }),
        }),
        expect.objectContaining({
          selection: expect.objectContaining({
            metric_key: "git.lines_added",
            display_dimensions: [],
          }),
        }),
      ]),
      { title: "Commits & Lines added" }
    );
  });

  it("opens a grouped point with its exact period and dimensions", async () => {
    const user = userEvent.setup();
    const byKey = timeseriesByKey();
    const metric = byKey.get("git.commits");
    if (!metric) throw new Error("missing fixture metric");
    metric.drilldown = { granularity: ["event"] };
    metric.selection = {
      metric_key: metric.metric_key,
      entity: { type: "person", ids: [ENTITY_ID] },
      period: RANGE,
      filters: [],
    };
    mocks.collection.mockReturnValue({ ...ready, byKey });
    mocks.evidenceColumn = groupedTimeseriesModel().columns[0]?.key ?? "";
    const openEvidenceTargets = vi.fn();
    render(
      <EvidenceDialogContext.Provider
        value={{
        openEvidence: vi.fn(),
        openEvidenceTargets,
        openEvidencePeople: vi.fn(),
      }}
      >
        <MetricTimeseriesView
          id="point-evidence"
          entityId={ENTITY_ID}
          range={RANGE}
          metricKeys={["git.commits"]}
          groupBy={{ default: "repository" }}
        />
      </EvidenceDialogContext.Provider>
    );

    await user.click(screen.getByRole("button", { name: "drill point" }));
    expect(openEvidenceTargets).toHaveBeenCalledWith(
      [
        {
          selection: expect.objectContaining({
            metric_key: "git.commits",
            period: { from: "2026-04-20", to: "2026-04-26" },
            filters: [
              {
                dimension: "repository",
                values: ["org/repo-a"],
              },
            ],
            display_dimensions: ["repository"],
          }),
          label: "Commits",
        },
      ],
      { activeMetricKey: "git.commits" }
    );
  });
});
