// @vitest-environment jsdom
/**
 * What a person actually did, at the closest grain the metric offers.
 *
 * Three grains render three different things from the same component, so what
 * these tests pin is that each one is drawn as itself: a counter is never
 * dressed up as a list of things, a ratio shows the denominator it was taken
 * out of, and a metric with no detail says so rather than leaving a hole that
 * reads as a failed load.
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import { normalizeMetricResults } from "@/lib/metrics/collection";

const detail = vi.hoisted(() => ({
  state: {
    isPending: false,
    isError: false,
    data: undefined as unknown,
    refetch: () => {},
  },
  calls: [] as unknown[],
}));
vi.mock("@/queries/metric-detail", () => ({
  DETAIL_LIMIT: 200,
  useMetricDetail: (selection: unknown, enabled?: boolean) => {
    detail.calls.push({ selection, enabled });
    return detail.state;
  },
}));

// The catalogue decides whether `source` may be asked for alongside a git
// metric's evidence; these tests are about what the grains RENDER, so it
// answers with a declaration and no request of its own.
const declared = vi.hoisted(() => ({
  byMetricKey: new Map<string, ReadonlySet<string>>(),
}));
vi.mock("@/queries/metric-definitions", () => ({
  useDeclaredMetricDimensions: () => ({
    byMetricKey: declared.byMetricKey,
    isPending: false,
  }),
}));

import { MetricActivity } from "./metric-activity";

const ME = "019e27bc-dec0-7626-81a9-c5524662a6a9";

function metric(
  key: string,
  granularity: string[] | null,
  value: number | null,
  over: Partial<MetricResult> = {}
) {
  const result = {
    metric_key: key,
    label: key === "git.commits" ? "Commits" : "Messages Sent",
    description: "what this counts",
    unit: key === "git.commits" ? "commits" : "messages",
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    views: [{ view: "period", values: [{ entity_id: ME, value }] }],
    selection: {
      metric_key: key,
      entity: { type: "person", ids: [ME] },
      period: { from: "2026-03-01", to: "2026-03-05" },
      filters: [],
    },
    ...(granularity ? { drilldown: { granularity } } : {}),
    ...over,
  } as unknown as MetricResult;
  return normalizeMetricResults([result]).get(key)!;
}

function draw(m: ReturnType<typeof metric>) {
  return render(
    <MetricActivity
      metric={m}
      previous={null}
      entityId={ME}
      periodNoun="month"
    />
  );
}

beforeEach(() => {
  detail.calls = [];
  detail.state = {
    isPending: false,
    isError: false,
    data: undefined,
    refetch: () => {},
  };
});

describe("MetricActivity", () => {
  it("says a metric offers no detail rather than leaving a hole", () => {
    // A reader who can open the day of every other metric on the page reads
    // silence here as a screen that failed to load.
    draw(metric("git.commits", null, 14));
    expect(screen.getByText(/reports a period total only/)).toBeInTheDocument();
  });

  it("does not ask for detail a metric cannot give", () => {
    draw(metric("git.commits", null, 14));
    expect(detail.calls).toHaveLength(1);
    expect((detail.calls[0] as { enabled: boolean }).enabled).toBe(false);
  });

  it("lists the things themselves at event grain", () => {
    detail.state.data = {
      columns: [],
      rows: [
        {
          values: {
            date: "2026-03-04",
            title: "Fix the thing\n\nbody",
            repository: "example/app",
            ref: "abc",
          },
        },
        {
          values: {
            date: "2026-03-02",
            title: "Another change",
            repository: "example/app",
            ref: "def",
          },
        },
      ],
    };
    draw(metric("git.commits", ["event"], 14));
    // Newest first, and only the subject of the message.
    const rows = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(rows[0]).toContain("Fix the thing");
    expect(rows[0]).not.toContain("body");
    expect(rows[1]).toContain("Another change");
  });

  it("draws days at counter grain, and names the days with no reading", () => {
    detail.state.data = {
      columns: [
        { key: "date", label: "Date", type: "date" },
        { key: "value", label: "Value", type: "number" },
      ],
      // Two of the five days in the period are missing entirely.
      rows: [
        { values: { date: "2026-03-01", value: 4 } },
        { values: { date: "2026-03-03", value: 2 } },
        { values: { date: "2026-03-05", value: 0 } },
      ],
    };
    const { container } = draw(
      metric("collab.messages_sent", ["source_summary"], 6)
    );
    expect(screen.getByText(/2 days with no reading/)).toBeInTheDocument();
    // Nothing is listed — a counter is not a list of things.
    expect(container.querySelector("li")).toBeNull();
  });

  it("names a constant denominator, because that is what a share is argued with", () => {
    detail.state.data = {
      columns: [
        { key: "date", label: "Date", type: "date" },
        { key: "numerator", label: "Numerator", type: "number" },
        { key: "denominator", label: "Denominator", type: "number" },
      ],
      rows: [
        { values: { date: "2026-03-01", numerator: 6, denominator: 8 } },
        { values: { date: "2026-03-02", numerator: 7, denominator: 8 } },
      ],
    };
    draw(metric("collab.focus_time_pct", ["derived_population"], 81));
    expect(screen.getByText(/measured against 8 per day/)).toBeInTheDocument();
  });

  it("offers a retry when the detail could not be loaded", () => {
    detail.state.isError = true;
    draw(metric("collab.messages_sent", ["source_summary"], 6));
    expect(
      screen.getByRole("button", { name: /try again/i })
    ).toBeInTheDocument();
  });

  it("describes the strip for anyone not reading it with their eyes", () => {
    // Thirty-one focusable days per strip is a worse answer for a keyboard
    // reader than one sentence saying what the shape is.
    detail.state.data = {
      columns: [
        { key: "date", label: "Date", type: "date" },
        { key: "value", label: "Value", type: "number" },
      ],
      rows: [
        { values: { date: "2026-03-01", value: 4 } },
        { values: { date: "2026-03-03", value: 9 } },
      ],
    };
    draw(metric("collab.messages_sent", ["source_summary"], 13));
    const label = screen.getByRole("img").getAttribute("aria-label") ?? "";
    expect(label).toMatch(/Messages Sent by day/);
    expect(label).toMatch(/busiest/);
    expect(label).toMatch(/3 days have no reading/);
  });

  it("anchors the day readout to the edge of the day it describes", () => {
    // The readout grows away from the nearer end so it cannot run off the
    // strip, and which edge it hangs from has to follow: one growing leftwards
    // ENDS at its day's right boundary. Anchoring both directions to the left
    // boundary put every readout in the second half a full day to the left of
    // the day under the pointer.
    detail.state.data = {
      columns: [
        { key: "date", label: "Date", type: "date" },
        { key: "value", label: "Value", type: "number" },
      ],
      rows: [
        { values: { date: "2026-03-01", value: 4 } },
        { values: { date: "2026-03-05", value: 9 } },
      ],
    };
    const { container } = draw(
      metric("collab.messages_sent", ["source_summary"], 13)
    );
    const bars = container.querySelectorAll<HTMLElement>('[role="img"] > div');
    expect(bars).toHaveLength(5);
    const readout = () =>
      container.querySelector<HTMLElement>('[role="img"] > [aria-hidden]')!;

    fireEvent.pointerEnter(bars[0]!);
    expect(readout().style.left).toBe("0%");
    expect(readout().style.transform).toBe("");

    // The last of five days: its right boundary is the strip's own end.
    fireEvent.pointerEnter(bars[4]!);
    expect(readout().style.left).toBe("100%");
    expect(readout().style.transform).toBe("translateX(-100%)");
  });

  it("says nothing was recorded rather than drawing an empty chart", () => {
    detail.state.data = { columns: [], rows: [] };
    draw(metric("collab.messages_sent", ["source_summary"], 0));
    expect(
      screen.getByText(/Nothing recorded in this period/)
    ).toBeInTheDocument();
  });
});
