// @vitest-environment jsdom
/**
 * Platform usage. The load-bearing part is that every number on it is dated:
 * a bar the reader cannot put a day against says traffic happened, not when.
 */
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Recharts needs a real layout; the assertions here are about what the page
// hands the chart, so the primitives report their props as data attributes.
vi.mock("@/components/ui/chart", () => ({
  ChartContainer: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="chart">{children}</div>
  ),
  BarChart: ({
    data,
    children,
  }: {
    data: unknown[];
    children: React.ReactNode;
  }) => (
    <div data-testid="plot" data-rows={JSON.stringify(data)}>
      {children}
    </div>
  ),
  ChartBar: ({ dataKey }: { dataKey: string }) => (
    <div data-testid="bar" data-key={dataKey} />
  ),
  XAxis: ({ dataKey }: { dataKey: string }) => (
    <div data-testid="x-axis" data-key={dataKey} />
  ),
  YAxis: () => <div data-testid="y-axis" />,
  CartesianGrid: () => <div data-testid="grid" />,
  ChartTooltip: () => <div data-testid="tooltip" />,
  ChartTooltipContent: () => <div data-testid="tooltip-content" />,
}));

// The house harness for a virtualized body: report every row, and record what
// the page asked to virtualize.
const mocks = vi.hoisted(() => ({ counts: [] as number[], summary: null as unknown }));
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => {
    mocks.counts.push(count);
    return {
      getVirtualItems: () =>
        Array.from({ length: count }, (_, index) => ({ index, start: index * 44 })),
      getTotalSize: () => count * 44,
      measureElement: () => {},
    };
  },
}));

vi.mock("@/queries/identity-me", () => ({
  useIsAdmin: () => ({ isAdmin: true, isPending: false }),
}));

const SUMMARY = {
  since: "2026-08-01",
  until: "2026-08-03",
  totals: { visits: 5, visitors: 2, page_views: 9 },
  by_day: [
    { day: "2026-08-01", visits: 3, visitors: 2 },
    { day: "2026-08-02", visits: 0, visitors: 0 },
    { day: "2026-08-03", visits: 2, visitors: 1 },
  ],
  by_person: [],
  by_page: [],
  by_event: [],
};

const asked: Array<{ since?: string; until?: string }> = [];

vi.mock("@/queries/usage", () => ({
  useUsageSummary: (range: { since?: string; until?: string }) => {
    asked.push(range);
    return { data: mocks.summary ?? SUMMARY, isPending: false, isError: false };
  },
}));

// The shared period control the rest of the portal uses; here it only has to
// report what the page hands it.
vi.mock("@/components/widgets/period-selector-bar", () => ({
  PeriodSelectorBar: ({ period }: { period: string }) => (
    <div data-testid="period-bar" data-period={period} />
  ),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueries: () => [],
}));

import { PlatformUsage } from "./platform-usage";

describe("PlatformUsage", () => {
  beforeEach(() => {
    mocks.summary = null;
    mocks.counts.length = 0;
  });

  it("asks for a period with the same control every other zone uses", () => {
    render(<PlatformUsage />);

    expect(screen.getByTestId("period-bar")).toHaveAttribute("data-period", "month");
    const range = asked.at(-1);
    expect(range?.since).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(range?.until).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("counts today, which the shared period window leaves out", () => {
    render(<PlatformUsage />);

    const today = new Date().toISOString().slice(0, 10);
    expect(asked.at(-1)?.until).toBe(today);
  });

  it("hands a long breakdown to the virtualizer instead of rendering all of it", () => {
    const many = Array.from({ length: 300 }, (_, i) => ({
      path: `/portal/page-${i}`,
      views: 300 - i,
      visitors: 1,
    }));
    mocks.summary = { ...SUMMARY, by_page: many };
    render(<PlatformUsage />);

    expect(mocks.counts).toContain(300);
  });

  it("plots visits against the day they happened", () => {
    render(<PlatformUsage />);

    expect(screen.getByTestId("x-axis")).toHaveAttribute("data-key", "day");
    expect(screen.getByTestId("bar")).toHaveAttribute("data-key", "visits");

    const rows = JSON.parse(
      screen.getByTestId("plot").getAttribute("data-rows") ?? "[]",
    ) as Array<{ day: string; visits: number }>;
    expect(rows.map((row) => row.day)).toEqual([
      "2026-08-01",
      "2026-08-02",
      "2026-08-03",
    ]);
  });
});
