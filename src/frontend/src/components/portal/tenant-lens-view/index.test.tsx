// @vitest-environment jsdom
/**
 * Tenant lens (issue #2803). The load-bearing parts: the request rides a
 * TENANT entity (a person id anywhere in it would be a cross-grain leak), the
 * whole tab collapses honestly when nothing is observed, and vendor words the
 * reader would misread (a superseded deployment marked "inactive") are
 * relabeled at the section that shows them.
 */
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { TenantLensConfig } from "@/lib/portal/lens-configs";
import type {
  MetricCollectionConfig,
  MetricCollectionEntity,
  NormalizedMetricResult,
} from "@/lib/metrics/collection";

const mocks = vi.hoisted(() => ({
  calls: [] as Array<{
    collection: MetricCollectionConfig;
    entity: MetricCollectionEntity;
  }>,
  result: {
    byKey: new Map<string, NormalizedMetricResult>(),
    previousByKey: null as Map<string, NormalizedMetricResult> | null,
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: () => {},
  },
  trends: [] as Array<{ title: string; series: unknown[]; data: unknown[] }>,
}));

vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: (
    collection: MetricCollectionConfig,
    entity: MetricCollectionEntity
  ) => {
    mocks.calls.push({ collection, entity });
    return mocks.result;
  },
}));

vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    period: "month",
    dateRange: { from: "2026-03-01", to: "2026-03-31" },
  }),
}));

vi.mock("@/components/portal/section-trend", () => ({
  SectionTrend: (props: { title: string; series: unknown[]; data: unknown[] }) => {
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
  ChartBar: () => <div />,
  XAxis: () => <div />,
  YAxis: () => <div />,
  CartesianGrid: () => <div />,
  ChartTooltip: () => <div />,
  ChartTooltipContent: () => <div />,
}));

import { TenantLensView } from "./index";

const TENANT = "11111111-1111-1111-1111-111111111111";

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

const CONFIG: TenantLensConfig = {
  title: "Development · CI",
  tagline: "pipelines, org-wide",
  entity: "tenant",
  notIngested: "No CI runs collected yet.",
  sections: [
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
  ],
};

beforeEach(() => {
  mocks.calls.length = 0;
  mocks.trends.length = 0;
  mocks.result.byKey = new Map();
  mocks.result.previousByKey = null;
  mocks.result.isPending = false;
  mocks.result.isError = false;
});

describe("TenantLensView", () => {
  it("requests the lens over a tenant entity with per-section views", () => {
    render(<TenantLensView config={CONFIG} />);

    expect(mocks.calls).toHaveLength(1);
    expect(mocks.calls[0].entity).toEqual({ type: "tenant" });

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
        metric("ci.deployments", {
          breakdown: {
            view: "breakdown",
            dimensions: ["environment", "outcome"],
            values: [
              {
                entity_id: TENANT,
                value: 3,
                dimensions: [
                  { key: "environment", value: "production" },
                  { key: "outcome", value: "inactive" },
                ],
              },
              {
                entity_id: TENANT,
                value: 2,
                dimensions: [
                  { key: "environment", value: "preview-1" },
                  { key: "outcome", value: "success" },
                ],
              },
            ],
          },
        }),
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
        metric("ci.deployments", {
          breakdown: {
            view: "breakdown",
            dimensions: ["environment", "outcome"],
            values: [
              {
                entity_id: TENANT,
                value: 3,
                dimensions: [
                  { key: "environment", value: "production" },
                  { key: "outcome", value: "success" },
                ],
              },
              {
                entity_id: TENANT,
                value: 2,
                dimensions: [
                  { key: "environment", value: "production" },
                  { key: "outcome", value: "failure" },
                ],
              },
            ],
          },
        }),
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
        metric("ci.deployments", {
          breakdown: {
            view: "breakdown",
            dimensions: ["environment", "outcome"],
            values: [
              {
                entity_id: TENANT,
                value: 3,
                dimensions: [
                  { key: "environment", value: "production" },
                  { key: "outcome", value: "success" },
                ],
              },
            ],
          },
        }),
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
        metric("ci.runs", {
          timeseries: {
            view: "timeseries",
            bucket: "month",
            series: [
              {
                entity_id: TENANT,
                dimensions: [],
                points: [{ bucket_start: "2026-03-01", value: 5 }],
              },
            ],
          },
        }),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(mocks.trends).toHaveLength(0);
  });

  it("charts the org series once it has at least two buckets", () => {
    mocks.result.byKey = new Map([
      [
        "ci.runs",
        metric("ci.runs", {
          label: "CI runs",
          timeseries: {
            view: "timeseries",
            bucket: "month",
            series: [
              {
                entity_id: TENANT,
                dimensions: [],
                points: [
                  { bucket_start: "2026-02-01", value: 4 },
                  { bucket_start: "2026-03-01", value: 5 },
                ],
              },
            ],
          },
        }),
      ],
    ]);

    render(<TenantLensView config={CONFIG} />);
    expect(mocks.trends).toHaveLength(1);
    expect(mocks.trends[0].data).toEqual([
      { date: "2026-02-01", "ci.runs": 4 },
      { date: "2026-03-01", "ci.runs": 5 },
    ]);
  });
});
