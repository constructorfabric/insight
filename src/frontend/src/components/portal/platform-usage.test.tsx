// @vitest-environment jsdom
/**
 * Platform usage. The load-bearing part is that every number on it is dated:
 * a bar the reader cannot put a day against says traffic happened, not when.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

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

vi.mock("@/queries/usage", () => ({
  useUsageSummary: () => ({
    data: SUMMARY,
    isPending: false,
    isError: false,
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueries: () => [],
}));

import { PlatformUsage } from "./platform-usage";

describe("PlatformUsage", () => {
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
