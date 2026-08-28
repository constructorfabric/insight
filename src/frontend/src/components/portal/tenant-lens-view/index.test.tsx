// @vitest-environment jsdom
/**
 * Tenant lens (issue #2803). The load-bearing parts: the request rides a
 * TENANT entity (a person id anywhere in it would be a cross-grain leak), the
 * whole tab collapses honestly when nothing is observed, conflicting views
 * spread over extra collections and half-window requests (plan.ts), and
 * vendor words the reader would misread are relabeled at the section that
 * shows them.
 */
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DateRange } from "@/api/period-to-date-range";
import type { TenantLensConfig } from "@/lib/portal/lens-configs";
import type {
  MetricCollectionConfig,
  MetricCollectionEntity,
  NormalizedMetricResult,
} from "@/lib/metrics/collection";

type HookResult = {
  byKey: Map<string, NormalizedMetricResult>;
  previousByKey: Map<string, NormalizedMetricResult> | null;
  isPending: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: () => void;
};

function emptyResult(): HookResult {
  return {
    byKey: new Map(),
    previousByKey: null,
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: () => {},
  };
}

const PORTAL_RANGE = { from: "2026-03-01", to: "2026-03-31" };

const mocks = vi.hoisted(() => ({
  calls: [] as Array<{
    collection: MetricCollectionConfig;
    entity: MetricCollectionEntity;
    range: DateRange;
    compareTo: DateRange | undefined;
  }>,
  setCalls: [] as Array<{
    collections: ReadonlyArray<{ key: string; collection: MetricCollectionConfig }>;
    entity: MetricCollectionEntity;
    range: DateRange;
  }>,
  result: null as unknown as HookResult,
  /** Half-window results, keyed by "<from>..<to>". */
  rangeResults: new Map<string, HookResult>(),
  /** Extra-collection results, keyed by the collection key ("extra-1"…). */
  setResults: new Map<string, HookResult>(),
  trends: [] as Array<{
    title: string;
    series: Array<{ key: string; label: string; type?: string }>;
    data: Array<Record<string, unknown>>;
    targetLine?: { value: number; label: string };
  }>,
  scatters: [] as Array<{ data: unknown[] }>,
}));

vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: (
    collection: MetricCollectionConfig,
    entity: MetricCollectionEntity,
    range: DateRange,
    options?: { compareTo?: DateRange }
  ) => {
    const compareTo = options?.compareTo;
    mocks.calls.push({ collection, entity, range, compareTo });
    // Both halves come from one call now: the range is the second half and the
    // comparison window the first, each answered from the same fixtures.
    if (compareTo) {
      const result = emptyResult();
      result.byKey =
        mocks.rangeResults.get(`${range.from}..${range.to}`)?.byKey ?? new Map();
      result.previousByKey =
        mocks.rangeResults.get(`${compareTo.from}..${compareTo.to}`)?.byKey ??
        new Map();
      return result;
    }
    return range.from === PORTAL_RANGE.from && range.to === PORTAL_RANGE.to
      ? mocks.result
      : emptyResult();
  },
  useMetricCollectionSet: (
    collections: ReadonlyArray<{ key: string; collection: MetricCollectionConfig }>,
    entity: MetricCollectionEntity,
    range: DateRange
  ) => {
    mocks.setCalls.push({ collections, entity, range });
    return new Map(
      collections.map(({ key }) => [key, mocks.setResults.get(key) ?? emptyResult()])
    );
  },
  collectionSetPending: (set: Map<string, HookResult>) =>
    [...set.values()].some((result) => result.isPending),
}));

vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    period: "month",
    dateRange: { from: "2026-03-01", to: "2026-03-31" },
  }),
}));

vi.mock("@/components/portal/section-trend", () => ({
  SectionTrend: (props: {
    title: string;
    series: Array<{ key: string; label: string; type?: string }>;
    data: Array<Record<string, unknown>>;
    targetLine?: { value: number; label: string };
  }) => {
    mocks.trends.push(props);
    return <div data-testid="trend" data-title={props.title} />;
  },
}));

vi.mock("@/components/ui/chart", () => ({
  ChartContainer: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="chart">{children}</div>
  ),
  BarChart: ({ data, children }: { data: unknown[]; children: React.ReactNode }) => (
    <div data-testid="plot" data-rows={JSON.stringify(data)}>
      {children}
    </div>
  ),
  ComposedChart: ({
    data,
    children,
  }: {
    data: unknown[];
    children: React.ReactNode;
  }) => (
    <div data-testid="composed" data-rows={JSON.stringify(data)}>
      {children}
    </div>
  ),
  ScatterChart: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="scatter">{children}</div>
  ),
  ChartScatter: (props: { data: unknown[] }) => {
    mocks.scatters.push({ data: props.data });
    return <div />;
  },
  ChartBar: () => <div />,
  ChartLine: () => <div />,
  ZAxis: () => <div />,
  XAxis: () => <div />,
  YAxis: () => <div />,
  CartesianGrid: () => <div />,
  ReferenceArea: () => <div />,
  ReferenceLine: () => <div />,
  ChartTooltip: () => <div />,
  ChartTooltipContent: () => <div />,
}));

import { TenantLensView } from "./index";

const TENANT = "11111111-1111-1111-1111-111111111111";
const FIRST_HALF = "2026-03-01..2026-03-15";
const SECOND_HALF = "2026-03-16..2026-03-31";

function metric(
  key: string,
  views: Partial<NormalizedMetricResult>
): NormalizedMetricResult {
  return {
    metric_key: key,
    label: key,
    unit: null,
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    ...views,
  };
}

function breakdownOf(
  key: string,
  dimensions: string[],
  values: Array<{
    dims: Array<{ key: string; value: string; label?: string }>;
    value: number;
  }>,
  extra: Partial<NormalizedMetricResult> = {}
): NormalizedMetricResult {
  return metric(key, {
    ...extra,
    breakdown: {
      view: "breakdown",
      dimensions,
      values: values.map(({ dims, value }) => ({
        entity_id: TENANT,
        value,
        dimensions: dims,
      })),
    },
  });
}

function seriesOf(
  key: string,
  bucket: "day" | "week" | "month",
  series: Array<{
    dims: Array<{ key: string; value: string; label?: string }>;
    points: Array<[string, number]>;
  }>,
  extra: Partial<NormalizedMetricResult> = {}
): NormalizedMetricResult {
  return metric(key, {
    ...extra,
    timeseries: {
      view: "timeseries",
      bucket,
      series: series.map(({ dims, points }) => ({
        entity_id: TENANT,
        dimensions: dims,
        points: points.map(([bucket_start, value]) => ({ bucket_start, value })),
      })),
    },
  });
}

function lens(sections: TenantLensConfig["sections"]): TenantLensConfig {
  return {
    title: "Development · CI",
    tagline: "pipelines, org-wide",
    entity: "tenant",
    notIngested: "No CI runs collected yet.",
    sections,
  };
}

const CONFIG = lens([
  { kind: "headline", metrics: ["ci.gate_pass_rate"] },
  {
    kind: "trend",
    title: "Runs per week",
    metrics: ["ci.runs"],
  },
  {
    kind: "composition",
    metric: "ci.deployments",
    dimension: "environment",
    splitBy: "outcome",
    title: "Deployments by environment",
    segmentLabels: { inactive: "superseded" },
  },
  {
    kind: "histogram",
    metric: "ci.run_duration_min",
    title: "How long a gate run takes",
    caption: "Each bar is a duration range.",
  },
]);

beforeEach(() => {
  mocks.calls.length = 0;
  mocks.setCalls.length = 0;
  mocks.trends.length = 0;
  mocks.scatters.length = 0;
  mocks.result = emptyResult();
  mocks.rangeResults.clear();
  mocks.setResults.clear();
});

describe("TenantLensView", () => {
  it("requests the lens over a tenant entity with per-section views", () => {
    render(<TenantLensView config={CONFIG} />);

    // Main + the halves hook (disabled: empty collection).
    expect(mocks.calls).toHaveLength(2);
    for (const call of mocks.calls) {
      expect(call.entity).toEqual({ type: "tenant" });
    }
    expect(mocks.calls[1].collection.metrics).toHaveLength(0);
    // No conflicting views → no extra collections.
    expect(mocks.setCalls[0].collections).toHaveLength(0);

    const byKey = new Map(
      mocks.calls[0].collection.metrics.map((m) => [m.key, m.views])
    );
    expect(byKey.get("ci.gate_pass_rate")).toEqual([{ view: "period" }]);
    // One entity → the finest bucket fits; a month window charts by day.
    expect(byKey.get("ci.runs")).toEqual([
      { view: "timeseries", bucket: "day" },
    ]);
    expect(byKey.get("ci.deployments")).toEqual([
      { view: "breakdown", dimensions: ["environment", "outcome"] },
    ]);
    expect(byKey.get("ci.run_duration_min")).toEqual([{ view: "histogram" }]);
  });

  it("collapses to the not-ingested state when no metric is observed", () => {
    render(<TenantLensView config={CONFIG} />);
    expect(screen.getByText("No CI runs collected yet.")).toBeTruthy();
  });

  it("collapses to the not-ingested state on the views a data-less tenant gets", () => {
    // Exactly what the backend answers for an installation with the flag ON
    // and no CI rows at all: the period view zero-fills the requested entity
    // with a null value, and a non-dimensioned timeseries is SEEDED with one
    // all-null series per entity. Treating either as "observed" leaves the
    // reader a bare title over sections that all self-suppress.
    const dataless = (key: string) =>
      metric(key, {
        period: { view: "period", values: [{ entity_id: TENANT, value: null }] },
        timeseries: {
          view: "timeseries",
          bucket: "day",
          series: [
            {
              entity_id: TENANT,
              dimensions: [],
              points: [
                { bucket_start: "2026-03-01", value: null },
                { bucket_start: "2026-03-02", value: null },
              ],
            },
          ],
        },
      });
    mocks.result.byKey = new Map([
      ["ci.gate_pass_rate", dataless("ci.gate_pass_rate")],
      ["ci.runs", dataless("ci.runs")],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(screen.getByText("No CI runs collected yet.")).toBeTruthy();
    expect(mocks.trends).toHaveLength(0);
  });

  it("counts a single non-null bucket as observed", () => {
    // The mirror of the case above: one real reading anywhere in the lens is
    // enough to render it, even when every other view came back empty.
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        seriesOf("ci.runs", "day", [
          {
            dims: [],
            points: [["2026-03-01", 4]],
          },
        ]),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(screen.queryByText("No CI runs collected yet.")).toBeNull();
  });

  it("renders the org-wide headline value with its previous-period delta", () => {
    mocks.result.byKey = new Map([
      [
        "ci.gate_pass_rate",
        metric("ci.gate_pass_rate", {
          label: "Gate pass rate",
          computation: "ratio",
          format: "percent",
          period: {
            view: "period",
            values: [{ entity_id: TENANT, value: 88 }],
          },
        }),
      ],
    ]);
    mocks.result.previousByKey = new Map([
      [
        "ci.gate_pass_rate",
        metric("ci.gate_pass_rate", {
          period: {
            view: "period",
            values: [{ entity_id: TENANT, value: 80 }],
          },
        }),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(screen.getByText("88%")).toBeTruthy();
    expect(screen.getByText("org-wide")).toBeTruthy();
    // The delta is relative to the previous period (88 vs 80 → +10%).
    expect(screen.getByText("+10%")).toBeTruthy();
  });

  it("relabels the vendor's 'inactive' deployment segment as superseded", () => {
    mocks.result.byKey = new Map([
      [
        "ci.deployments",
        breakdownOf("ci.deployments", ["environment", "outcome"], [
          {
            dims: [
              { key: "environment", value: "production" },
              { key: "outcome", value: "inactive" },
            ],
            value: 3,
          },
          {
            dims: [
              { key: "environment", value: "preview-1" },
              { key: "outcome", value: "success" },
            ],
            value: 2,
          },
        ]),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(screen.getByText(/superseded/)).toBeTruthy();
    expect(screen.queryByText(/inactive/)).toBeNull();
  });

  it("keeps a single composition row when a real split cuts it", () => {
    // One environment with both outcomes: one bar, two meaningful segments —
    // the degenerate-data guard must not hide it.
    mocks.result.byKey = new Map([
      [
        "ci.deployments",
        breakdownOf("ci.deployments", ["environment", "outcome"], [
          {
            dims: [
              { key: "environment", value: "production" },
              { key: "outcome", value: "success" },
            ],
            value: 3,
          },
          {
            dims: [
              { key: "environment", value: "production" },
              { key: "outcome", value: "failure" },
            ],
            value: 2,
          },
        ]),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(screen.getByText("Deployments by environment")).toBeTruthy();
    expect(screen.getByText("production")).toBeTruthy();
  });

  it("still suppresses a single unsplit composition row", () => {
    mocks.result.byKey = new Map([
      [
        "ci.deployments",
        breakdownOf("ci.deployments", ["environment", "outcome"], [
          {
            dims: [
              { key: "environment", value: "production" },
              { key: "outcome", value: "success" },
            ],
            value: 3,
          },
        ]),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(screen.queryByText("Deployments by environment")).toBeNull();
  });

  it("charts the served histogram bins and counts the runs in the title", () => {
    mocks.result.byKey = new Map([
      [
        "ci.run_duration_min",
        metric("ci.run_duration_min", {
          label: "Gate run duration",
          unit: "min",
          computation: "median",
          format: "decimal",
          histogram: {
            view: "histogram",
            values: [
              {
                entity_id: TENANT,
                bins: [
                  { lo: 0, hi: 2, count: 5 },
                  { lo: 2, hi: 4, count: 3 },
                ],
              },
            ],
          },
        }),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(screen.getByText(/How long a gate run takes · 8 runs/)).toBeTruthy();
    const plot = screen.getByTestId("plot");
    const rows = JSON.parse(plot.getAttribute("data-rows") ?? "[]") as Array<{
      count: number;
    }>;
    expect(rows.map((r) => r.count)).toEqual([5, 3]);
  });

  it("suppresses an empty histogram rather than drawing an empty plot", () => {
    mocks.result.byKey = new Map([
      [
        "ci.run_duration_min",
        metric("ci.run_duration_min", {
          histogram: {
            view: "histogram",
            values: [{ entity_id: TENANT, bins: [] }],
          },
        }),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(screen.queryByText(/How long a gate run takes/)).toBeNull();
  });

  it("suppresses a trend that has fewer than two buckets", () => {
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        seriesOf("ci.runs", "month", [
          { dims: [], points: [["2026-03-01", 5]] },
        ]),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(mocks.trends).toHaveLength(0);
  });

  it("charts the org series once it has at least two buckets", () => {
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        seriesOf(
          "ci.runs",
          "month",
          [
            {
              dims: [],
              points: [
                ["2026-02-01", 4],
                ["2026-03-01", 5],
              ],
            },
          ],
          { label: "CI runs" }
        ),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(mocks.trends).toHaveLength(1);
    expect(mocks.trends[0].data).toEqual([
      { date: "2026-02-01", "ci.runs": 4 },
      { date: "2026-03-01", "ci.runs": 5 },
    ]);
  });

  it("draws the window mean and flags trailing-mean outliers on a trend", () => {
    const steady: Array<[string, number]> = [
      ["2026-03-01", 96],
      ["2026-03-02", 95],
      ["2026-03-03", 97],
      ["2026-03-04", 96],
      ["2026-03-05", 60],
    ];
    mocks.result.byKey = new Map([
      [
        "ci.gate_pass_rate",
        seriesOf("ci.gate_pass_rate", "day", [{ dims: [], points: steady }]),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "trend",
            title: "Gate pass rate per day",
            metrics: ["ci.gate_pass_rate"],
            referenceMean: true,
            flagOutliers: true,
          },
        ])}
      />
    );
    expect(mocks.trends).toHaveLength(1);
    expect(mocks.trends[0].targetLine).toEqual({
      value: (96 + 95 + 97 + 96 + 60) / 5,
      label: "window mean",
    });
    expect(screen.getByText(/2026-03-05/)).toBeTruthy();
  });

  it("stacks a dimensioned series and converts shares when asked", () => {
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        seriesOf("ci.runs", "week", [
          {
            dims: [{ key: "outcome", value: "success" }],
            points: [
              ["2026-03-02", 6],
              ["2026-03-09", 2],
            ],
          },
          {
            dims: [{ key: "outcome", value: "failure" }],
            points: [
              ["2026-03-02", 2],
              ["2026-03-09", 2],
            ],
          },
        ]),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "stacked-trend",
            metric: "ci.runs",
            splitBy: "outcome",
            share: true,
            title: "Outcome mix as shares",
          },
        ])}
      />
    );
    expect(mocks.trends).toHaveLength(1);
    expect(mocks.trends[0].series.map((s) => s.type)).toEqual([
      "stacked-area",
      "stacked-area",
    ]);
    expect(mocks.trends[0].data[0]).toEqual({
      date: "2026-03-02",
      success: 75,
      failure: 25,
    });
  });

  it("routes a conflicting view to an extra collection and renders from it", () => {
    // The plain trend claims ci.runs' timeseries slot in collection 0; the
    // stacked trend's dimensioned twin must ride an extra collection.
    const config = lens([
      { kind: "trend", title: "Runs", metrics: ["ci.runs"] },
      {
        kind: "stacked-trend",
        metric: "ci.runs",
        splitBy: "outcome",
        title: "Outcomes",
      },
    ]);
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        seriesOf("ci.runs", "day", [
          {
            dims: [],
            points: [
              ["2026-03-01", 8],
              ["2026-03-02", 9],
            ],
          },
        ]),
      ],
    ]);
    const extra = emptyResult();
    extra.byKey = new Map([
      [
        "ci.runs",
        seriesOf("ci.runs", "day", [
          {
            dims: [{ key: "outcome", value: "success" }],
            points: [
              ["2026-03-01", 6],
              ["2026-03-02", 7],
            ],
          },
          {
            dims: [{ key: "outcome", value: "failure" }],
            points: [
              ["2026-03-01", 2],
              ["2026-03-02", 2],
            ],
          },
        ]),
      ],
    ]);
    mocks.setResults.set("extra-1", extra);

    render(<TenantLensView config={config} />);
    expect(mocks.setCalls[0].collections.map((c) => c.key)).toEqual(["extra-1"]);
    expect(mocks.setCalls[0].collections[0].collection.metrics).toEqual([
      {
        key: "ci.runs",
        views: [
          { view: "timeseries", bucket: "day", dimensions: ["outcome"] },
        ],
      },
    ]);
    expect(mocks.trends).toHaveLength(2);
    expect(mocks.trends[1].data[0]).toEqual({
      date: "2026-03-01",
      success: 6,
      failure: 2,
    });
  });

  it("renders small multiples ranked by volume under one shared ceiling", () => {
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        seriesOf("ci.runs", "day", [
          {
            dims: [{ key: "repository", value: "org/big", label: "org/big" }],
            points: [
              ["2026-03-01", 30],
              ["2026-03-02", 40],
            ],
          },
          {
            dims: [{ key: "repository", value: "org/small", label: "org/small" }],
            points: [
              ["2026-03-01", 1],
              ["2026-03-02", 2],
            ],
          },
        ]),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "small-multiples",
            metric: "ci.runs",
            dimension: "repository",
            title: "Weekly runs per repository",
            top: 12,
          },
        ])}
      />
    );
    expect(screen.getByText("Weekly runs per repository")).toBeTruthy();
    expect(screen.getByText("org/big")).toBeTruthy();
    expect(screen.getByText("org/small")).toBeTruthy();
  });

  it("scatters joined breakdowns and suppresses under three points", () => {
    const dims = (value: string) => [
      { key: "repository", value, label: value },
    ];
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        breakdownOf("ci.runs", ["repository"], [
          { dims: dims("a"), value: 10 },
          { dims: dims("b"), value: 20 },
          { dims: dims("c"), value: 30 },
        ]),
      ],
      [
        "ci.gate_pass_rate",
        breakdownOf(
          "ci.gate_pass_rate",
          ["repository"],
          [
            { dims: dims("a"), value: 90 },
            { dims: dims("b"), value: 80 },
            { dims: dims("c"), value: 70 },
          ],
          { computation: "ratio", format: "percent" }
        ),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "scatter",
            x: "ci.runs",
            y: "ci.gate_pass_rate",
            dimension: "repository",
            title: "Volume against reliability",
            quadrants: true,
          },
        ])}
      />
    );
    expect(screen.getByText("Volume against reliability")).toBeTruthy();
    expect(mocks.scatters).toHaveLength(1);
    expect(mocks.scatters[0].data).toHaveLength(3);
  });

  it("folds a day-bucketed hour_block series into the weekday heatmap", () => {
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        seriesOf("ci.runs", "day", [
          {
            dims: [{ key: "hour_block", value: "08", label: "08–10" }],
            // 2026-03-02 is a Monday.
            points: [["2026-03-02", 7]],
          },
        ]),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          { kind: "heatmap-hours", metric: "ci.runs", title: "When CI runs" },
        ])}
      />
    );
    expect(screen.getByText("When CI runs")).toBeTruthy();
    expect(screen.getByTitle("Mon 08:00 — 7")).toBeTruthy();
    expect(screen.getByText(/7 total/)).toBeTruthy();
  });

  it("draws hour columns with the mean band", () => {
    const block = (value: string, label: string, rate: number) => ({
      dims: [{ key: "hour_block", value, label }],
      value: rate,
    });
    mocks.result.byKey = new Map([
      [
        "ci.gate_pass_rate",
        breakdownOf(
          "ci.gate_pass_rate",
          ["hour_block"],
          [
            block("00", "00–02", 90),
            block("08", "08–10", 85),
            block("10", "10–12", 88),
            block("22", "22–24", 60),
          ],
          { computation: "ratio", format: "percent" }
        ),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "hour-columns",
            metric: "ci.gate_pass_rate",
            title: "Is CI riskier at some hours?",
          },
        ])}
      />
    );
    const plot = screen.getByTestId("plot");
    const rows = JSON.parse(plot.getAttribute("data-rows") ?? "[]") as Array<{
      block: string;
      value: number;
    }>;
    expect(rows.map((r) => r.block)).toEqual(["00", "08", "10", "22"]);
  });

  it("requests half windows for slope/momentum and renders the deltas", () => {
    const config = lens([
      {
        kind: "slope",
        metric: "ci.gate_pass_rate",
        dimension: "repository",
        title: "First half against second",
      },
      {
        kind: "momentum",
        metric: "ci.gate_pass_rate",
        dimension: "repository",
        title: "Momentum",
      },
    ]);
    const rate = (value: string, rateValue: number) => ({
      dims: [{ key: "repository", value, label: value }],
      value: rateValue,
    });
    const firstHalf = emptyResult();
    firstHalf.byKey = new Map([
      [
        "ci.gate_pass_rate",
        breakdownOf(
          "ci.gate_pass_rate",
          ["repository"],
          [rate("org/a", 80), rate("org/b", 90)],
          { computation: "ratio", format: "percent" }
        ),
      ],
    ]);
    const secondHalf = emptyResult();
    secondHalf.byKey = new Map([
      [
        "ci.gate_pass_rate",
        breakdownOf(
          "ci.gate_pass_rate",
          ["repository"],
          [rate("org/a", 95), rate("org/b", 88)],
          { computation: "ratio", format: "percent" }
        ),
      ],
    ]);
    mocks.rangeResults.set(FIRST_HALF, firstHalf);
    mocks.rangeResults.set(SECOND_HALF, secondHalf);

    render(<TenantLensView config={config} />);
    // Both halves come from ONE request: the second half is its period and the
    // first its comparison window, so nothing computes a third aggregate over
    // the whole range that no section reads.
    expect(mocks.calls).toHaveLength(2);
    expect(mocks.calls[1].range).toEqual({ from: "2026-03-16", to: "2026-03-31" });
    expect(mocks.calls[1].compareTo).toEqual({ from: "2026-03-01", to: "2026-03-15" });
    expect(mocks.calls[1].collection.metrics).toEqual([
      {
        key: "ci.gate_pass_rate",
        views: [{ view: "breakdown", dimensions: ["repository"] }],
      },
    ]);
    expect(screen.getByText("Momentum")).toBeTruthy();
    // org/a moved +15 pts — labelled on both the slope list and momentum bars.
    expect(screen.getAllByText(/\+15\.0 pts/).length).toBeGreaterThanOrEqual(2);
    expect(screen.getAllByText(/−2\.0 pts/).length).toBeGreaterThanOrEqual(2);
  });

  it("computes what fixing the worst pipelines buys from ci.runs rows", () => {
    const row = (
      pipeline: string,
      trigger: string,
      outcome: string,
      value: number
    ) => ({
      dims: [
        { key: "pipeline", value: pipeline, label: pipeline },
        { key: "trigger", value: trigger },
        { key: "outcome", value: outcome },
      ],
      value,
    });
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        breakdownOf("ci.runs", ["pipeline", "trigger", "outcome"], [
          row("flaky", "push", "success", 10),
          row("flaky", "push", "failure", 10),
          row("solid", "push", "success", 80),
        ]),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          { kind: "marginal-impact", title: "What fixing would buy" },
        ])}
      />
    );
    expect(screen.getByText("90.0%")).toBeTruthy();
    expect(screen.getByText(/\+10\.0 pts/)).toBeTruthy();
    expect(screen.getByText(/flaky/)).toBeTruthy();
  });

  it("pairs the org headline with the unweighted per-group mean", () => {
    mocks.result.byKey = new Map([
      [
        "ci.gate_pass_rate",
        breakdownOf(
          "ci.gate_pass_rate",
          ["repository"],
          [
            { dims: [{ key: "repository", value: "a" }], value: 90 },
            { dims: [{ key: "repository", value: "b" }], value: 50 },
          ],
          {
            computation: "ratio",
            format: "percent",
            period: { view: "period", values: [{ entity_id: TENANT, value: 88 }] },
          }
        ),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "callout-pair",
            metric: "ci.gate_pass_rate",
            dimension: "repository",
            title: "Headline versus typical repository",
          },
        ])}
      />
    );
    expect(screen.getByText("88%")).toBeTruthy();
    expect(screen.getByText("70%")).toBeTruthy();
    expect(screen.getByText(/unweighted mean over 2 groups/)).toBeTruthy();
  });

  it("draws dumbbells only for values with both split readings", () => {
    const row = (pipeline: string, outcome: string, value: number) => ({
      dims: [
        { key: "pipeline", value: pipeline, label: pipeline },
        { key: "outcome", value: outcome },
      ],
      value,
    });
    mocks.result.byKey = new Map([
      [
        "ci.run_duration_min",
        breakdownOf(
          "ci.run_duration_min",
          ["pipeline", "outcome"],
          [
            row("slow", "failure", 30),
            row("slow", "success", 10),
            row("fast", "failure", 2),
            row("fast", "success", 12),
            row("half", "success", 7),
          ],
          { computation: "median", format: "decimal", unit: "min" }
        ),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "dumbbell",
            metric: "ci.run_duration_min",
            dimension: "pipeline",
            splitBy: "outcome",
            left: "failure",
            right: "success",
            title: "Fail fast or fail slow?",
          },
        ])}
      />
    );
    expect(screen.getByText("slow")).toBeTruthy();
    expect(screen.getByText("fast")).toBeTruthy();
    expect(screen.queryByText("half")).toBeNull();
  });

  it("ranks cumulative shares and reports the tail", () => {
    const rows = Array.from({ length: 14 }, (_, i) => ({
      dims: [{ key: "pipeline", value: `p${String(i).padStart(2, "0")}` }],
      value: 100 - i,
    }));
    mocks.result.byKey = new Map([
      [
        "ci.run_hours",
        breakdownOf("ci.run_hours", ["pipeline"], rows, {
          format: "decimal",
          unit: "h",
        }),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "cumulative",
            metric: "ci.run_hours",
            dimension: "pipeline",
            title: "Compute concentration",
          },
        ])}
      />
    );
    expect(screen.getByText("p00")).toBeTruthy();
    expect(screen.getByText(/\+2 more sharing the remaining/)).toBeTruthy();
  });

  it("decomposes a summable metric into one labelled 100% bar", () => {
    mocks.result.byKey = new Map([
      [
        "ci.run_hours",
        breakdownOf(
          "ci.run_hours",
          ["outcome"],
          [
            { dims: [{ key: "outcome", value: "success" }], value: 75 },
            { dims: [{ key: "outcome", value: "failure" }], value: 25 },
          ],
          { format: "decimal", unit: "h" }
        ),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "decomposition",
            metric: "ci.run_hours",
            splitBy: "outcome",
            title: "Where the CI hours went",
          },
        ])}
      />
    );
    expect(screen.getByText(/success/)).toBeTruthy();
    expect(screen.getByText(/\(75\.0%\)/)).toBeTruthy();
  });

  it("judges weekly stability and leaves thin histories out", () => {
    const weeks = (values: number[]): Array<[string, number]> =>
      values.map((value, i) => [`2026-0${i + 1}-05`, value]);
    mocks.result.byKey = new Map([
      [
        "ci.gate_pass_rate",
        seriesOf(
          "ci.gate_pass_rate",
          "week",
          [
            {
              dims: [{ key: "pipeline", value: "steady", label: "steady" }],
              points: weeks([97, 96, 98, 97, 96]),
            },
            {
              dims: [{ key: "pipeline", value: "wild", label: "wild" }],
              points: weeks([95, 40, 90, 30, 85]),
            },
            {
              dims: [{ key: "pipeline", value: "new", label: "new" }],
              points: weeks([90, 91]),
            },
          ],
          { computation: "ratio", format: "percent" }
        ),
      ],
    ]);

    render(
      <TenantLensView
        config={lens([
          {
            kind: "verdict-table",
            metric: "ci.gate_pass_rate",
            dimension: "pipeline",
            title: "Stability verdict per pipeline",
            minWeeks: 5,
          },
        ])}
      />
    );
    expect(screen.getByText("solid")).toBeTruthy();
    expect(screen.getByText("erratic")).toBeTruthy();
    expect(screen.queryByText("new")).toBeNull();
    expect(screen.getByText(/1 with under 5 weeks of history/)).toBeTruthy();
  });

  it("surfaces an error from any collection with a shared retry", () => {
    mocks.result = { ...emptyResult(), isError: true };

    render(<TenantLensView config={CONFIG} />);
    expect(
      screen.getByText(/Development · CI — unable to load/)
    ).toBeTruthy();
  });
});
