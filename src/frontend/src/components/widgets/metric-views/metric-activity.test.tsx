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
  collectedThrough: null as string | null,
  revisionWindowDays: null as number | null,
}));
vi.mock("@/queries/metric-detail", () => ({
  DETAIL_LIMIT: 200,
  useMetricDetail: (selection: unknown, enabled?: boolean) => {
    detail.calls.push({ selection, enabled });
    return detail.state;
  },
}));

// The strip reads the metric's own daily series, not evidence rows: evidence is
// paged, and a page covers the days it reaches rather than the period.
const series = vi.hoisted(() => ({
  state: {
    isPending: false,
    isError: false,
    data: undefined as unknown,
    refetch: () => {},
  },
  calls: [] as unknown[],
}));
vi.mock("@/queries/metric-day-series", () => ({
  useMetricDaySeries: (selection: unknown, enabled?: boolean) => {
    series.calls.push({ selection, enabled });
    return series.state;
  },
}));

// The catalogue decides whether `source` may be asked for alongside a git
// metric's evidence; these tests are about what the grains RENDER, so it
// answers with a declaration and no request of its own.
const declared = vi.hoisted(() => ({
  byMetricKey: new Map<string, ReadonlySet<string>>(),
}));
// One mock for the whole catalogue module: a second `vi.mock` of the same
// specifier replaces the first, so both hooks are served from here. The
// collection boundary is set per test through `detail.collectedThrough`.
vi.mock("@/queries/metric-definitions", () => ({
  useCollectedThrough: () => ({
    collectedThrough: detail.collectedThrough,
    revisionWindowDays: detail.revisionWindowDays,
  }),
  useDeclaredMetricDimensions: () => ({
    byMetricKey: declared.byMetricKey,
    isPending: false,
  }),
}));

import { MetricActivity } from "./metric-activity";

const ME = "019e27bc-dec0-7626-81a9-c5524662a6a9";

function reading(date: string, value: number) {
  return { date, value, numerator: null, denominator: null };
}

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
  declared.byMetricKey = new Map();
  detail.calls = [];
  detail.collectedThrough = null;
  detail.revisionWindowDays = null;
  detail.state = {
    isPending: false,
    isError: false,
    data: undefined,
    refetch: () => {},
  };
  series.calls = [];
  series.state = {
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
          links: {
            title: "https://git.example/example/app/commit/abc",
            repository: "https://git.example/example/app",
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
    expect(screen.getByRole("link", { name: "Fix the thing" })).toHaveAttribute(
      "href",
      "https://git.example/example/app/commit/abc"
    );
  });

  it("names the kind of issue beside each one", () => {
    // A count of closed issues reads differently once the bugs in it are
    // visible, and the tracker's own type is the only thing on a row that says
    // which is which.
    declared.byMetricKey.set("tasks.closed", new Set(["source", "type"]));
    detail.state.data = {
      columns: [],
      rows: [
        {
          values: {
            date: "2026-03-04",
            title: "Login fails on retry",
            ref: "example/app#12",
            type: "Bug",
          },
        },
      ],
    };
    draw(metric("tasks.closed", ["event"], 1));
    const row = screen.getAllByRole("listitem")[0]?.textContent ?? "";
    expect(row).toContain("Bug");
    expect(row).toContain("Login fails on retry");
    // Asked for on the read itself, or the rows would never carry it.
    const asked = detail.calls.at(-1) as {
      selection: { display_dimensions: string[] };
    };
    expect(asked.selection.display_dimensions).toContain("type");
  });

  it("spends no width on a type where the records have none", () => {
    detail.state.data = {
      columns: [],
      rows: [
        {
          values: {
            date: "2026-03-04",
            title: "Fix the thing",
            repository: "example/app",
            ref: "abc",
          },
        },
      ],
    };
    draw(metric("git.commits", ["event"], 1));
    expect(screen.getAllByRole("listitem")[0]?.textContent).not.toContain("—");
  });

  it("draws days at counter grain, and names the days with no reading", () => {
    // Two of the five days in the period are missing entirely.
    series.state.data = [
      reading("2026-03-01", 4),
      reading("2026-03-03", 2),
      reading("2026-03-05", 0),
    ];
    const { container } = draw(
      metric("collab.messages_sent", ["source_summary"], 6)
    );
    expect(screen.getByText(/2 days with no reading/)).toBeInTheDocument();
    // Nothing is listed — a counter is not a list of things.
    expect(container.querySelector("li")).toBeNull();
  });

  it("separates the days nobody collected from the days that were quiet", () => {
    // The source stops on the 3rd; the 4th and 5th were never delivered. Read
    // as silence they would say this person did none of it, which is the one
    // thing the data cannot support.
    detail.collectedThrough = "2026-03-03";
    series.state.data = [reading("2026-03-01", 4), reading("2026-03-03", 2)];
    draw(metric("collab.messages_sent", ["source_summary"], 6));
    // The 2nd is the only quiet day; the tail is uncollected, not quiet.
    expect(screen.getByText(/2 days not collected yet/)).toBeInTheDocument();
    expect(screen.queryByText(/days with no reading/)).not.toBeInTheDocument();
    const label = screen.getByRole("img").getAttribute("aria-label") ?? "";
    expect(label).toMatch(/1 day has no reading/);
    expect(label).toMatch(/2 days are not collected yet/);
  });

  it("marks the days the supplier may still revise without hiding their figures", () => {
    // Delivered through the 3rd, revised for 2 days: the 2nd and 3rd carry real
    // readings that are not final. Drawing them as absent would throw away a
    // measurement; drawing them as settled would overstate it.
    detail.collectedThrough = "2026-03-03";
    detail.revisionWindowDays = 2;
    series.state.data = [
      reading("2026-03-01", 4),
      reading("2026-03-02", 3),
      reading("2026-03-03", 2),
    ];
    draw(metric("collab.messages_sent", ["source_summary"], 6));
    const label = screen.getByRole("img").getAttribute("aria-label") ?? "";
    expect(label).toMatch(/2 days may still change/);
    // The uncollected tail still outranks it in the one-line caption.
    expect(screen.getByText(/2 days not collected yet/)).toBeInTheDocument();
  });

  it("names a constant denominator, because that is what a share is argued with", () => {
    // Percentages as the metric computes them — scale already applied.
    series.state.data = [
      { date: "2026-03-01", value: 75, numerator: 6, denominator: 8 },
      { date: "2026-03-02", value: 87.5, numerator: 7, denominator: 8 },
    ];
    // A real ratio, because the readout is scaled on the way out: a metric
    // left as a sum would render the same string whether the day value was a
    // share or a percentage already multiplied by 100.
    draw(
      metric("collab.focus_time_pct", ["derived_population"], 81, {
        computation: "ratio",
        format: "percent",
        scale: 100,
        unit: null,
      })
    );
    expect(screen.getByText(/measured against 8 per day/)).toBeInTheDocument();
    const label = screen.getByRole("img").getAttribute("aria-label") ?? "";
    // Formatted once, from the metric's own figure: 88%, not 8,750%.
    expect(label).toMatch(/busiest 2 Mar — 88% of 8/);
  });

  it("offers a retry when the list of things could not be loaded", () => {
    // The other error case covers the day strip; an event-grain section reads
    // a different query, and its failure has its own path to the same button.
    detail.state.isError = true;
    draw(metric("git.commits", ["event"], 3));
    expect(
      screen.getByRole("button", { name: /try again/i })
    ).toBeInTheDocument();
  });

  it("reads one source per grain and never both", () => {
    // Two queries hang off this component and only the one the grain renders
    // may fire; the other would be a round trip for rows nothing draws.
    draw(metric("collab.messages_sent", ["source_summary"], 6));
    expect((detail.calls.at(-1) as { enabled: boolean }).enabled).toBe(false);
    expect((series.calls.at(-1) as { enabled: boolean }).enabled).toBe(true);

    detail.calls = [];
    series.calls = [];
    draw(metric("git.commits", ["event"], 3));
    expect((detail.calls.at(-1) as { enabled: boolean }).enabled).toBe(true);
    expect((series.calls.at(-1) as { enabled: boolean }).enabled).toBe(false);
  });

  it("offers a retry when the detail could not be loaded", () => {
    series.state.isError = true;
    draw(metric("collab.messages_sent", ["source_summary"], 6));
    expect(
      screen.getByRole("button", { name: /try again/i })
    ).toBeInTheDocument();
  });

  it("describes the strip for anyone not reading it with their eyes", () => {
    // Thirty-one focusable days per strip is a worse answer for a keyboard
    // reader than one sentence saying what the shape is.
    series.state.data = [reading("2026-03-01", 4), reading("2026-03-03", 9)];
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
    series.state.data = [reading("2026-03-01", 4), reading("2026-03-05", 9)];
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
    series.state.data = [];
    draw(metric("collab.messages_sent", ["source_summary"], 0));
    expect(
      screen.getByText(/Nothing recorded in this period/)
    ).toBeInTheDocument();
  });
});
