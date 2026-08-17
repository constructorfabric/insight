// @vitest-environment jsdom
/**
 * One activity class, composed.
 *
 * The choices this file makes are all about what NOT to draw: which metrics
 * earn a detail block, when a whole class collapses to a sentence, and what the
 * index is allowed to repeat. Each of those is invisible in the output when it
 * works, which is why they are pinned here.
 */
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import { normalizeMetricResults } from "@/lib/metrics/collection";

const mocks = vi.hoisted(() => ({
  byKey: new Map<string, unknown>(),
  isPending: false,
  isError: false,
  cohort: [] as string[],
  definitions: [] as unknown[],
  definitionsPending: false,
  definitionsError: false,
}));

vi.mock("@/queries/metric-definitions", () => ({
  useMetricDefinitionsResponse: () => ({
    data: mocks.definitionsError ? undefined : { metrics: mocks.definitions },
    isPending: mocks.definitionsPending,
    isError: mocks.definitionsError,
  }),
}));
vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: () => ({
    byKey: mocks.byKey,
    previousByKey: null,
    isPending: mocks.isPending,
    isFetching: false,
    isError: mocks.isError,
    refetch: vi.fn(),
  }),
}));
vi.mock("@/lib/portal/use-person-cohort", () => ({
  usePersonCohort: () => mocks.cohort,
}));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    period: "month",
    dateRange: { from: "2026-03-01", to: "2026-03-31" },
  }),
}));
// The composition blocks are someone else's tests; stubbing them keeps this
// file about which children get built, not how they paint.
vi.mock("@/components/widgets/metric-views/collection-drilldown", () => ({
  CollectionDrilldown: ({ children }: { children?: React.ReactNode }) => (
    <div data-testid="composition">{children}</div>
  ),
}));
vi.mock("@/components/widgets/metric-views/metric-activity", () => ({
  MetricActivity: ({ metric }: { metric: { label: string } }) => (
    <div data-testid="activity">{metric.label}</div>
  ),
}));

import { SingleGroupView } from "./single-group-view";

const ME = "019e27bc-dec0-7626-81a9-c5524662a6a9";

function metric(key: string, label: string, value: number | null) {
  return {
    metric_key: key,
    label,
    unit: null,
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    views: [{ view: "period", values: [{ entity_id: ME, value }] }],
    drilldown: { granularity: ["source_summary"] },
  } as unknown as MetricResult;
}

/** Collaboration's headline keys, from the group registry. */
const HEADLINE = [
  ["collab.messages_sent", "Messages Sent"],
  ["collab.meeting_hours", "Meeting Hours"],
  ["collab.focus_time_pct", "Focus Time"],
] as const;

function draw() {
  return render(<SingleGroupView personId={ME} groupId="collaboration" />);
}

/** A listing entry for a metric this installation reads. */
function wired(key: string) {
  return {
    metric_key: key,
    is_enabled: true,
    schema_status: "ok",
    origin: "builtin",
    last_observed_date: "2026-03-20",
  };
}

beforeEach(() => {
  mocks.byKey = new Map();
  mocks.isPending = false;
  mocks.isError = false;
  mocks.cohort = [];
  // The section's sources are connected unless a test says otherwise: an
  // empty section is then about the person, which is the ordinary case.
  mocks.definitions = HEADLINE.map(([k]) => wired(k));
  mocks.definitionsPending = false;
  mocks.definitionsError = false;
});

describe("SingleGroupView", () => {
  it("collapses a class with nothing recorded into one sentence", () => {
    // Every composition block would otherwise render its own polite
    // placeholder — a chart, a summary card, a distribution — and a reader
    // would meet one fact restated four ways down a full page.
    mocks.byKey = normalizeMetricResults(
      HEADLINE.map(([k, l]) => metric(k, l, null))
    );
    draw();
    expect(screen.getByText(/Nothing recorded here/)).toBeInTheDocument();
    expect(screen.queryByTestId("composition")).not.toBeInTheDocument();
  });

  it("says nothing feeds a section rather than blaming the person for it", () => {
    // The two empties are opposite findings. A section this person did none
    // of is worth asking them about; one nothing feeds is about the install
    // and is nobody's performance. Told the second in the first's words, a
    // reader goes looking for missing work that was never being measured.
    mocks.byKey = normalizeMetricResults(
      HEADLINE.map(([k, l]) => metric(k, l, null))
    );
    mocks.definitions = [];
    draw();
    expect(screen.getByText(/No data source is connected for this section/)).toBeInTheDocument();
    expect(screen.queryByText(/Nothing recorded here/)).not.toBeInTheDocument();
  });

  it("claims neither absence when the listing that decides could not be read", () => {
    // With no listing every section looks unreachable, so falling through
    // would announce that nothing is measured here for anyone on the strength
    // of a request that never arrived.
    mocks.byKey = normalizeMetricResults(
      HEADLINE.map(([k, l]) => metric(k, l, null))
    );
    mocks.definitionsError = true;
    draw();
    expect(screen.getByText(/Nothing to show here/)).toBeInTheDocument();
    expect(screen.queryByText(/No data source is connected/)).not.toBeInTheDocument();
  });

  it("waits for the listing rather than showing one sentence and swapping it", () => {
    mocks.byKey = normalizeMetricResults(
      HEADLINE.map(([k, l]) => metric(k, l, null))
    );
    mocks.definitionsPending = true;
    draw();
    expect(screen.queryByText(/Nothing recorded here/)).not.toBeInTheDocument();
    expect(screen.queryByText(/No data source is connected/)).not.toBeInTheDocument();
  });

  it("gives a detail block only to the headline metrics that read", () => {
    mocks.byKey = normalizeMetricResults([
      metric("collab.messages_sent", "Messages Sent", 400),
      metric("collab.meeting_hours", "Meeting Hours", null),
      metric("collab.focus_time_pct", "Focus Time", 81),
    ]);
    draw();
    const shown = screen.getAllByTestId("activity").map((n) => n.textContent);
    expect(shown).toEqual(["Messages Sent", "Focus Time"]);
  });

  it("says so once when the class reads but nothing can be broken down", () => {
    // Stated for the class rather than under every metric: three rows each
    // saying "no detail" reads as three separate faults.
    const flat = HEADLINE.map(([k, l]) => {
      const m = metric(k, l, 5) as unknown as Record<string, unknown>;
      delete m.drilldown;
      return m as unknown as MetricResult;
    });
    mocks.byKey = normalizeMetricResults(flat);
    draw();
    expect(screen.getByText(/report period totals only/)).toBeInTheDocument();
    expect(screen.queryAllByTestId("activity")).toHaveLength(0);
  });

  it("shows a spinner while the collection loads, not an empty class", () => {
    // A class still loading must not be drawn as one with nothing; the reader
    // would take the sentence for an answer.
    mocks.isPending = true;
    const { container } = draw();
    expect(screen.queryByText(/Nothing recorded here/)).not.toBeInTheDocument();
    expect(container.querySelector("svg")).toBeInTheDocument();
  });

  it("surfaces a failed fetch as retryable rather than as an empty class", () => {
    mocks.isError = true;
    draw();
    expect(screen.queryByText(/Nothing recorded here/)).not.toBeInTheDocument();
    expect(screen.getByRole("button")).toBeInTheDocument();
  });

  it("refuses a group it does not know", () => {
    render(<SingleGroupView personId={ME} groupId={"not_a_group" as never} />);
    expect(screen.getByText(/Unknown group/)).toBeInTheDocument();
  });
});
