// @vitest-environment jsdom
/**
 * Platform usage. The load-bearing part is that every number on it is dated:
 * a bar the reader cannot put a day against says traffic happened, not when.
 */
import { fireEvent, render, screen } from "@testing-library/react";
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
const mocks = vi.hoisted(() => ({
  counts: [] as number[],
  summary: null as unknown,
  asked: [] as Array<{ since: string; until: string }>,
  feedbackAsked: [] as Array<{ since: string; until: string }>,
  feedback: null as unknown,
}));
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
  useUsageSummary: (range: { since: string; until: string }) => {
    mocks.asked.push(range);
    return { data: mocks.summary ?? SUMMARY, isPending: false, isError: false };
  },
}));

const FEEDBACK = {
  since: "2026-08-01",
  until: "2026-08-03",
  items: [
    {
      feedback_id: "f-1",
      ts: "2026-08-02 11:20:00",
      person_id: "00000000-0000-0000-0000-0000000000aa",
      display_name: "Alice Example",
      username: "alice",
      message: "The cohort control does not say what it compares against.",
      path: "/portal/overview",
    },
  ],
};

vi.mock("@/queries/feedback", () => ({
  useFeedbackList: (range: { since: string; until: string }) => {
    mocks.feedbackAsked.push(range);
    return {
      data: mocks.feedback ?? FEEDBACK,
      isPending: false,
      isError: false,
    };
  },
}));

// The shared period control the rest of the portal uses; here it only has to
// report what the page hands it.
vi.mock("@/components/widgets/period-selector-bar", () => ({
  PeriodSelectorBar: ({
    period,
    onPeriodChange,
    onRangeChange,
  }: {
    period: string;
    onPeriodChange: (next: string) => void;
    onRangeChange: (range: { from: string; to: string } | null) => void;
  }) => (
    <div data-testid="period-bar" data-period={period}>
      <button
        data-testid="pick-custom"
        onClick={() => onRangeChange({ from: "2026-01-01", to: "2026-01-07" })}
      />
      <button data-testid="pick-week" onClick={() => onPeriodChange("week")} />
    </div>
  ),
}));

import { resolveDateRange } from "@/api/period-to-date-range";

import { PlatformUsage } from "./platform-usage";

describe("PlatformUsage", () => {
  beforeEach(() => {
    mocks.summary = null;
    mocks.feedback = null;
    mocks.counts.length = 0;
    mocks.asked.length = 0;
    mocks.feedbackAsked.length = 0;
  });

  it("asks for a period with the same control every other zone uses", () => {
    render(<PlatformUsage />);

    expect(screen.getByTestId("period-bar")).toHaveAttribute("data-period", "month");
    expect(mocks.asked.at(-1)?.since).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("names a visitor by handle when the identity rows carry no display name", () => {
    mocks.summary = {
      ...SUMMARY,
      by_person: [
        {
          person_id: "4d1f0a6c-0000-4000-8000-0000000000aa",
          display_name: "",
          username: "ada",
          visits: 2,
          page_views: 3,
          last_seen: "2026-08-02T10:00:00Z",
        },
      ],
    };

    render(<PlatformUsage />);

    expect(screen.getByText("ada")).toBeInTheDocument();
    expect(screen.queryByText("4d1f0a6c-0000-4000-8000-0000000000aa")).toBeNull();
  });

  it("counts today without stretching the period it names", () => {
    render(<PlatformUsage />);

    const asked = mocks.asked.at(-1)!;
    // The UTC day, not the reader's. The server buckets by UTC, so east of
    // Greenwich the reader's date names a day ClickHouse has no rows for yet,
    // and the chart draws it as a zero.
    expect(asked.until).toBe(new Date().toISOString().slice(0, 10));
    // The shared window ends yesterday; moving only its end would make a
    // 30-day month cover 31.
    const shared = resolveDateRange("month", null);
    const days = (range: { since: string; until: string }) =>
      (Date.parse(range.until) - Date.parse(range.since)) / 86_400_000;
    expect(days(asked)).toBe(days({ since: shared.from, until: shared.to }));
  });

  it("returns to a preset after a custom range", () => {
    render(<PlatformUsage />);

    fireEvent.click(screen.getByTestId("pick-custom"));
    expect(mocks.asked.at(-1)).toEqual({ since: "2026-01-01", until: "2026-01-07" });

    fireEvent.click(screen.getByTestId("pick-week"));
    const asked = mocks.asked.at(-1)!;
    // A custom range outranks the period in resolveDateRange, so a preset that
    // did not clear it would still be serving January.
    expect(asked.since).not.toBe("2026-01-01");
    const days = (Date.parse(asked.until) - Date.parse(asked.since)) / 86_400_000 + 1;
    expect(days).toBe(7);
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

  it("reads feedback over the very window the numbers above it cover", () => {
    render(<PlatformUsage />);

    expect(mocks.feedbackAsked.at(-1)).toEqual(mocks.asked.at(-1));
  });

  it("names who sent each piece of feedback", () => {
    render(<PlatformUsage />);

    expect(screen.getByText("Alice Example")).toBeInTheDocument();
    expect(
      screen.getByText("The cohort control does not say what it compares against."),
    ).toBeInTheDocument();
  });

  it("says so plainly when nobody sent anything in the period", () => {
    mocks.feedback = { since: "2026-08-01", until: "2026-08-03", items: [] };
    render(<PlatformUsage />);

    expect(screen.getByText("No feedback in this period")).toBeInTheDocument();
  });
});
