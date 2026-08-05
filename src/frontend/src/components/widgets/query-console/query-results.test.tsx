import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { ResultRow } from "@/lib/query-console/result-shape";

// Deterministic, DOM-inspectable stand-ins for the recharts wrappers (which
// render nothing under jsdom's zero-size ResponsiveContainer).
vi.mock("@/components/ui/chart", () => ({
  ChartContainer: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="chart">{children}</div>
  ),
  BarChart: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  ChartBar: ({ name }: { name: string }) => <div data-testid="bar">{name}</div>,
  CartesianGrid: () => null,
  XAxis: () => null,
  YAxis: () => null,
  ChartTooltip: () => null,
  ChartTooltipContent: () => null,
}));

import { QueryResults } from "./query-results";

describe("QueryResults", () => {
  it("shows an empty message for no rows and no chart", () => {
    render(<QueryResults rows={[]} />);
    expect(screen.getByText("The query returned no rows.")).toBeInTheDocument();
    expect(screen.queryByTestId("chart")).not.toBeInTheDocument();
  });

  it("renders inferred columns and cell values as a table", () => {
    const rows: ResultRow[] = [
      { tool: "github", commits: 10, note: null },
    ];
    render(<QueryResults rows={rows} />);
    expect(screen.getByText("tool")).toBeInTheDocument();
    expect(screen.getByText("commits")).toBeInTheDocument();
    expect(screen.getByText("github")).toBeInTheDocument();
    // Null cell renders as an em dash, not empty.
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("renders an auto-chart with a series per numeric column when chartable", () => {
    const rows: ResultRow[] = [
      { tool: "github", commits: 10, prs: 3 },
      { tool: "gitlab", commits: 7, prs: 2 },
    ];
    render(<QueryResults rows={rows} />);
    expect(screen.getByTestId("chart")).toBeInTheDocument();
    const bars = screen.getAllByTestId("bar").map((b) => b.textContent);
    expect(bars).toEqual(["commits", "prs"]);
  });

  it("renders table-only (no chart) for a non-chartable shape", () => {
    const rows: ResultRow[] = [{ a: 1, b: 2 }];
    render(<QueryResults rows={rows} />);
    expect(screen.queryByTestId("chart")).not.toBeInTheDocument();
    expect(screen.getByText("a")).toBeInTheDocument();
  });
});
