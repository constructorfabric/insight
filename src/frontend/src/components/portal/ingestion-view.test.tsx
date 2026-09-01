// @vitest-environment jsdom
/**
 * The Ingestion lens. Two things are load-bearing beyond "it renders": the
 * drill-down is a URL write (so a reload and a shared link reproduce it), and
 * the trend axis is configured so a one-row bucket is still a visible bar.
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { IngestionIntensity } from "@/api/ingestion-client";

// Recharts needs a real layout; every assertion here is about what the view
// HANDS the chart, so the primitives report their props as data attributes.
vi.mock("@/components/ui/chart", () => ({
  ChartContainer: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="chart">{children}</div>
  ),
  BarChart: ({ data, children }: { data: unknown[]; children: React.ReactNode }) => (
    <div data-testid="plot" data-rows={JSON.stringify(data)}>
      {children}
    </div>
  ),
  ChartBar: ({
    dataKey,
    stackId,
    fill,
  }: {
    dataKey: string;
    stackId?: string;
    fill?: string;
  }) => (
    <div data-testid="bar" data-key={dataKey} data-stack={stackId ?? ""} data-fill={fill} />
  ),
  XAxis: ({ dataKey, type, domain }: { dataKey: string; type?: string; domain?: unknown }) => (
    <div
      data-testid="x-axis"
      data-key={dataKey}
      data-type={type ?? ""}
      data-domain={JSON.stringify(domain ?? null)}
    />
  ),
  YAxis: ({ scale, domain, ticks }: { scale?: string; domain?: unknown; ticks?: unknown }) => (
    <div
      data-testid="y-axis"
      data-scale={scale ?? "linear"}
      data-domain={JSON.stringify(domain ?? null)}
      data-ticks={JSON.stringify(ticks ?? null)}
    />
  ),
  CartesianGrid: () => <div data-testid="grid" />,
  ChartTooltip: () => <div data-testid="tooltip" />,
  ChartTooltipContent: () => <div data-testid="tooltip-content" />,
  ChartLegend: () => <div data-testid="legend" />,
  ChartLegendContent: () => <div data-testid="legend-content" />,
}));

const mocks = vi.hoisted(() => ({
  conn: undefined as string | undefined,
  patches: [] as Array<Record<string, unknown>>,
  asked: [] as Array<Record<string, unknown>>,
  truncated: false,
}));

vi.mock("@/lib/portal/portal-search", () => ({
  usePortalSearch: () => ({ conn: mocks.conn }),
  useSetPortalSearch: () => (patch: Record<string, unknown>) => {
    mocks.patches.push(patch);
  },
}));

function reply(req: Record<string, unknown>): IngestionIntensity {
  const scope = (req.scope as string | null) ?? undefined;
  const series =
    (req.series as string | undefined) ?? (scope ? "stream" : "connector");
  const base = {
    grain: req.grain as IngestionIntensity["grain"],
    series: series as IngestionIntensity["series"],
    from: "2026-08-26T00:00:00.000Z",
    to: "2026-08-26T01:00:00.000Z",
    scope,
    truncated: mocks.truncated,
  };
  if (series === "total") {
    return {
      ...base,
      points: [
        // A one-row bucket: the case a zero-baselined log axis erases.
        { bucket: "2026-08-26 00:00:00", key: "all", rows: 1 },
        { bucket: "2026-08-26 00:45:00", key: "all", rows: 48_000 },
      ],
    };
  }
  if (series === "stream") {
    return {
      ...base,
      points: [
        { bucket: "2026-08-26 00:00:00", key: "issues", rows: 30 },
        { bucket: "2026-08-26 00:00:00", key: "_boards", rows: 2 },
      ],
    };
  }
  return {
    ...base,
    points: [
      { bucket: "2026-08-26 00:00:00", key: "jira", rows: 90 },
      { bucket: "2026-08-26 00:15:00", key: "slack", rows: 10 },
    ],
  };
}

vi.mock("@/queries/ingestion", () => ({
  useIngestionIntensity: (req: Record<string, unknown>) => {
    mocks.asked.push(req);
    return { isPending: false, isError: false, data: reply(req) };
  },
}));

const { IngestionView } = await import("@/components/portal/ingestion-view");

beforeEach(() => {
  mocks.conn = undefined;
  mocks.patches = [];
  mocks.asked = [];
  mocks.truncated = false;
});

describe("the overview", () => {
  it("plots every connector and offers each as a drill-down", () => {
    render(<IngestionView />);
    expect(screen.getByRole("heading", { name: "Ingestion" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /jira/ }));
    expect(mocks.patches).toEqual([{ conn: "jira" }]);
  });

  it("reads org-wide, with no scope on any request", () => {
    render(<IngestionView />);
    expect(mocks.asked).toHaveLength(3);
    expect(mocks.asked.every((req) => req.scope === null)).toBe(true);
    // The trend asks for a lookback, not a resolved instant: resolving one
    // here would mean reading the clock during a render.
    expect(mocks.asked[0]).toMatchObject({ grain: "15m", series: "total" });
    expect(mocks.asked[0].lookbackDays).toBeGreaterThan(0);
    expect(mocks.asked[0].from).toBeUndefined();
    // The other two omit the window so the server's per-grain default applies
    // and the query key stays stable while they refetch.
    expect(mocks.asked[1]).toMatchObject({ grain: "15m" });
    expect(mocks.asked[2]).toMatchObject({ grain: "1s" });
  });

  it("states that the timestamps are extraction time, not insert time", () => {
    // The number is honest only if the page says what it counts.
    render(<IngestionView />);
    expect(screen.getByText(/extraction intensity, not insert time/)).toBeTruthy();
    expect(screen.getByText(/not deduplicated/)).toBeTruthy();
  });
});

describe("the drill-down", () => {
  beforeEach(() => {
    mocks.conn = "jira";
  });

  it("names the connector and scopes every read to its bronze database", () => {
    render(<IngestionView />);
    expect(screen.getByRole("heading", { name: "jira" })).toBeTruthy();
    expect(mocks.asked.every((req) => req.scope === "bronze_jira")).toBe(true);
  });

  it("bands by stream", () => {
    render(<IngestionView />);
    expect(screen.getByText("Recent activity by stream")).toBeTruthy();
    const stacked = screen
      .getAllByTestId("bar")
      .filter((bar) => bar.getAttribute("data-stack") === "rows");
    expect(stacked.map((bar) => bar.getAttribute("data-key"))).toContain("issues");
    expect(stacked.map((bar) => bar.getAttribute("data-key"))).toContain("_boards");
  });

  it("goes back by clearing the key, not by pushing a bare address", () => {
    render(<IngestionView />);
    fireEvent.click(screen.getByRole("button", { name: /All connectors/ }));
    expect(mocks.patches).toEqual([{ conn: undefined }]);
  });

  it("drops the roster, which is an overview affordance", () => {
    render(<IngestionView />);
    expect(screen.queryByText(/Connectors active/)).toBeNull();
  });
});

describe("the axes", () => {
  it("floors the log axis below one so a single-row bucket has height", () => {
    render(<IngestionView />);
    const trend = screen.getAllByTestId("y-axis")[0];
    expect(trend.getAttribute("data-scale")).toBe("log");
    const domain = JSON.parse(trend.getAttribute("data-domain") ?? "null") as number[];
    expect(domain[0]).toBeLessThan(1);
    expect(domain[1]).toBe(48_000);
    // Whole powers of ten: Recharts' own log ticks would print the floor.
    expect(JSON.parse(trend.getAttribute("data-ticks") ?? "null")).toEqual([
      1, 10, 100, 1000, 10_000, 100_000,
    ]);
  });

  it("leaves the stacked axes linear", () => {
    render(<IngestionView />);
    const [, recent, live] = screen.getAllByTestId("y-axis");
    expect(recent.getAttribute("data-scale")).toBe("linear");
    expect(live.getAttribute("data-scale")).toBe("linear");
  });

  it("plots time numerically, so an idle hour reads as a gap", () => {
    render(<IngestionView />);
    const axis = screen.getAllByTestId("x-axis")[0];
    expect(axis.getAttribute("data-key")).toBe("epoch");
    expect(axis.getAttribute("data-type")).toBe("number");
    // Bounded by the window the SERVER reported, not by the plotted extremes:
    // a quiet tail must stay visible as empty space. Widened by half a bucket
    // at each end so the edge bars are not clipped.
    const half = 15 * 60 * 1_000 / 2;
    expect(JSON.parse(axis.getAttribute("data-domain") ?? "null")).toEqual([
      Date.parse("2026-08-26T00:00:00.000Z") - half,
      Date.parse("2026-08-26T01:00:00.000Z") + half,
    ]);
  });

  it("hands the plot UTC epochs, not re-cut local buckets", () => {
    render(<IngestionView />);
    const rows = JSON.parse(
      screen.getAllByTestId("plot")[0].getAttribute("data-rows") ?? "[]",
    ) as Array<{ epoch: number }>;
    expect(rows[0].epoch).toBe(Date.parse("2026-08-26T00:00:00Z"));
  });
});

describe("a clipped read", () => {
  it("says so rather than showing a short chart as complete", () => {
    mocks.truncated = true;
    render(<IngestionView />);
    expect(screen.getAllByText(/Clipped/).length).toBeGreaterThan(0);
  });
});
