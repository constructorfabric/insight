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
  MetricTimeseriesTable: () => <div>table presentation</div>,
}));

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
    render(
      <EvidenceDialogContext.Provider
        value={{ openEvidence, openEvidenceTargets }}
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
      "Commits & Lines added"
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
    const openEvidence = vi.fn();
    render(
      <EvidenceDialogContext.Provider
        value={{ openEvidence, openEvidenceTargets: vi.fn() }}
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
    expect(openEvidence).toHaveBeenCalledWith(
      expect.objectContaining({
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
      "Commits"
    );
  });
});
