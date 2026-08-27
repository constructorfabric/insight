// @vitest-environment jsdom
/**
 * AI & Cost zone semantics: the Claude-only cost caveat ("not tracked" is
 * never $0), adoption math (active users, funnel stage cuts), per-tool
 * aggregation from the breakdown view, by-unit rollups under a slice, and
 * honest ComingSoon for unwired pane items.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { act, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { identityPerson, pid } from "@/test/identity";
import type { IdentityPerson, TeamMember } from "@/types/insight";

const mocks = vi.hoisted(() => ({
  personId: null as string | null,
  tree: undefined as IdentityPerson | undefined,
  members: [] as TeamMember[],
  grid: {
    byKey: new Map<string, NormalizedMetricResult>(),
    previousByKey: new Map<string, NormalizedMetricResult>(),
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
  },
  tools: {
    byKey: new Map<string, NormalizedMetricResult>(),
    previousByKey: null,
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
  },
  monthly: {
    byKey: new Map<string, NormalizedMetricResult>(),
    previousByKey: null,
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
  },
}));

vi.mock("@/auth", () => ({
  useViewer: () => ({ email: "boss@x", personId: mocks.personId }),
}));
vi.mock("@/lib/portal/use-cohort-label", () => ({
  useCohortLabel: () => "team",
}));
vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({ data: mocks.tree, isPending: false, isLoading: false, isError: false, refetch: vi.fn() }),
}));
// `useOrgScope` reads the deployment's visibility policy, and its flat branch
// reads the roster. This suite is about the view, so both answer statically.
vi.mock("@/queries/identity-me", () => ({
  useVisibilityPolicy: () => ({
    policy: "org_chart",
    isFlat: false,
    isPending: false,
  }),
}));
vi.mock("@/queries/visible-roster", () => ({
  useVisibleRoster: () => ({
    roster: [],
    truncated: false,
    isPending: false,
    isError: false,
    retry: () => {},
  }),
}));
vi.mock("@/queries/team-view", () => ({
  useTeamMembers: () => ({ data: mocks.members, isPending: false, isLoading: false, isError: false, refetch: vi.fn() }),
}));
vi.mock("@/queries/member-grid", () => ({ useMemberGridData: () => mocks.grid }));
// The view takes two collections through one hook. Dispatching on the requested
// keys keeps them apart, so a failing per-tool request cannot stand in for a
// failing monthly one and each section is asserted on its own data.
vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: (collection: { metrics: Array<{ key: string }> }) =>
    collection.metrics.some((m) => m.key === "ai.seat_cost")
      ? mocks.monthly
      : mocks.tools,
}));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({ period: "week", dateRange: { from: "2026-07-20", to: "2026-07-26" } }),
}));


import { AiCostView } from "./ai-cost-view";

const person = (
  label: string,
  over: Partial<IdentityPerson> = {},
  subs: IdentityPerson[] = [],
): IdentityPerson => identityPerson(label, over, subs);

const member = (id: string): TeamMember =>
  ({ person_id: id, name: `Name ${id.split("@")[0]}` }) as unknown as TeamMember;

function metric(
  key: string,
  period: Array<[string, number | null]>,
  over: Partial<NormalizedMetricResult> = {},
): NormalizedMetricResult {
  return {
    metric_key: key,
    label: key,
    unit: null,
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    period: { view: "period", values: period.map(([entity_id, value]) => ({ entity_id, value })) },
    peer: { view: "peer", values: period.map(([entity_id, value]) => ({ entity_id, target_value: value })) },
    ...over,
  } as unknown as NormalizedMetricResult;
}

function toolBreakdown(
  key: string,
  rows: Array<[string, string, number]>,
): NormalizedMetricResult {
  return {
    ...metric(key, []),
    breakdown: {
      view: "breakdown",
      values: rows.map(([entity_id, tool, value]) => ({
        entity_id,
        dimensions: [{ key: "tool", value: tool }],
        value,
      })),
    },
  } as unknown as NormalizedMetricResult;
}

/** A month-bucketed result: one series per entity, one point per billing month. */
function monthlySeries(
  key: string,
  rows: Array<[string, string, number | null]>,
): NormalizedMetricResult {
  const byEntity = new Map<
    string,
    Array<{ bucket_start: string; value: number | null }>
  >();
  for (const [entity_id, bucket_start, value] of rows) {
    const points = byEntity.get(entity_id) ?? [];
    points.push({ bucket_start, value });
    byEntity.set(entity_id, points);
  }
  return {
    ...metric(key, []),
    format: "currency",
    unit: "USD",
    timeseries: {
      view: "timeseries",
      bucket: "month",
      series: [...byEntity].map(([entity_id, points]) => ({
        entity_id,
        dimensions: [],
        points,
      })),
    },
  } as unknown as NormalizedMetricResult;
}

/** The table row a label sits in — so a money assertion is scoped to one month. */
function row(label: string): HTMLElement {
  const found = screen.getByText(label).closest("tr");
  if (!found) throw new Error(`no table row holds ${label}`);
  return found as HTMLElement;
}

// Roster entity ids are person UUIDs (identity cutover).
const LABELS = ["a", "b", "c", "d"];
const IDS = LABELS.map(pid);

beforeEach(() => {
  mocks.personId = pid("boss");
  mocks.tree = person("boss", {}, LABELS.map((l) => person(l)));
  mocks.members = IDS.map(member);
  mocks.grid.isPending = false;
  mocks.grid.isError = false;
  mocks.tools.isError = false;
  // 3 of 4 use AI; costs are Claude-only. Actual (billed) cost is a fraction of
  // the potential one, which is the relationship the two figures always have.
  mocks.grid.byKey = new Map([
    ["ai.cost", metric("ai.cost", [[pid("a"), 100], [pid("b"), 50], [pid("c"), 0], [pid("d"), 0]], { format: "currency", unit: "USD" } as never)],
    ["ai.daily_approximate_extra_usage_cost", metric("ai.daily_approximate_extra_usage_cost", [[pid("a"), 8], [pid("b"), 4], [pid("c"), 0], [pid("d"), 0]], { format: "currency", unit: "USD" } as never)],
    ["ai.active_days", metric("ai.active_days", [[pid("a"), 5], [pid("b"), 3], [pid("c"), 1], [pid("d"), 0]])],
    ["ai.accepted_lines", metric("ai.accepted_lines", [[pid("a"), 700], [pid("b"), 200], [pid("c"), 100], [pid("d"), 0]])],
  ]);
  mocks.grid.previousByKey = new Map();
  mocks.tools.byKey = new Map([
    ["ai.cost", toolBreakdown("ai.cost", [[pid("a"), "claude_code", 100], [pid("b"), "claude_code", 50]])],
    // `claude`, the code seat billing actually reports — not `claude_code`.
    ["ai.daily_approximate_extra_usage_cost", toolBreakdown("ai.daily_approximate_extra_usage_cost", [
      [pid("a"), "claude", 8],
      [pid("b"), "claude", 4],
    ])],
    ["ai.accepted_lines", toolBreakdown("ai.accepted_lines", [
      [pid("a"), "claude_code", 600],
      [pid("b"), "chatgpt", 300],
      [pid("c"), "chatgpt", 100],
    ])],
  ]);
  mocks.monthly.isError = false;
  // Two billing months. July carries both facts — a $40 seat and a $160 one, and
  // the usage billed on top of them. June carries only billed usage: no invoice
  // priced its tier, which is absence rather than a $0 seat.
  mocks.monthly.byKey = new Map([
    ["ai.seat_cost", monthlySeries("ai.seat_cost", [
      [pid("a"), "2026-07-01", 40],
      [pid("b"), "2026-07-01", 160],
    ])],
    ["ai.extra_usage_cost", monthlySeries("ai.extra_usage_cost", [
      [pid("a"), "2026-07-01", 3],
      [pid("b"), "2026-07-01", 7],
      [pid("a"), "2026-06-01", 2],
    ])],
  ]);
  act(() => {
    portalRouter.set({ slice: undefined });
    portalRouter.set({ scope: undefined, direct: false });
  });
});

describe("AiCostView", () => {
  it("renders headline KPIs: Claude-only cost, active users, org lines", () => {
    render(<AiCostView item={null} />);
    expect(screen.getByText("AI potential usage cost")).toBeInTheDocument();
    expect(screen.getByText("Claude Code only")).toBeInTheDocument();
    // 3 of 4 members have active days > 0
    expect(screen.getByText("Active AI users")).toBeInTheDocument();
    expect(screen.getByText("75% of 4")).toBeInTheDocument();
    expect(screen.getByText(/1[,  ]?000/)).toBeInTheDocument(); // 700+200+100 lines
  });

  it("holds the actual cost beside the potential one rather than summing them", () => {
    render(<AiCostView item={null} />);
    // Potential 100+50, actual 8+4 — two figures, no 162 anywhere.
    expect(screen.getAllByText("$150").length).toBeGreaterThan(0);
    expect(screen.getByText("actual cost $12")).toBeInTheDocument();
    expect(screen.queryByText("$162")).not.toBeInTheDocument();
    // The average carries the same pair: 150/3 against 12/3, never 162/3.
    expect(screen.getAllByText("$50").length).toBeGreaterThan(0);
    expect(screen.getByText("avg actual $4 / active user")).toBeInTheDocument();
    expect(screen.queryByText("$54")).not.toBeInTheDocument();
  });

  it("omits the actual figure entirely when no seat data reaches us", () => {
    const drop = <T,>(m: Map<string, T>) =>
      new Map([...m].filter(([key]) => key !== "ai.daily_approximate_extra_usage_cost"));
    mocks.grid.byKey = drop(mocks.grid.byKey);
    mocks.tools.byKey = drop(mocks.tools.byKey);
    render(<AiCostView item={null} />);
    expect(screen.getByText("AI potential usage cost")).toBeInTheDocument();
    // A $0 actual cost would assert that nothing was billed.
    expect(screen.queryByText(/actual cost/)).not.toBeInTheDocument();
    expect(screen.queryByText(/actual .* \/ active user/)).not.toBeInTheDocument();
  });

  it("omits the actual figure when the metric is served but nobody has a reading", () => {
    // Served and empty is not the same as absent, and it is the state the daily
    // metric reaches whenever the window holds no reading for these people.
    mocks.grid.byKey = new Map([
      ...mocks.grid.byKey,
      [
        "ai.daily_approximate_extra_usage_cost",
        metric("ai.daily_approximate_extra_usage_cost", [], {
          format: "currency",
          unit: "USD",
        } as never),
      ],
    ]);
    mocks.tools.byKey = new Map([
      ...mocks.tools.byKey,
      [
        "ai.daily_approximate_extra_usage_cost",
        toolBreakdown("ai.daily_approximate_extra_usage_cost", []),
      ],
    ]);
    render(<AiCostView item={null} />);
    expect(screen.queryByText(/actual cost/)).not.toBeInTheDocument();
  });

  it("omits the potential figure too when nobody has a reading for it", () => {
    mocks.grid.byKey = new Map([
      ...mocks.grid.byKey,
      ["ai.cost", metric("ai.cost", [], { format: "currency", unit: "USD" } as never)],
    ]);
    // The per-tool cards read their own breakdown, so both reads go silent or
    // the figure survives in a card.
    mocks.tools.byKey = new Map([
      ...mocks.tools.byKey,
      ["ai.cost", toolBreakdown("ai.cost", [])],
    ]);
    render(<AiCostView item={null} />);
    // Neither the total nor the per-user average may be conjured from no reading.
    expect(screen.queryByText("$150")).not.toBeInTheDocument();
    expect(screen.queryByText("$50")).not.toBeInTheDocument();
    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  });

  it("shows per-tool cards where untracked cost reads 'not tracked', never $0", () => {
    render(<AiCostView item={null} />);
    expect(screen.getAllByText("Claude Code").length).toBeGreaterThan(0);
    expect(screen.getByText("ChatGPT")).toBeInTheDocument();
    // ChatGPT reports lines but no cost
    expect(screen.getByText("potential cost not tracked")).toBeInTheDocument();
    // ...and no billed amount either, so its card names neither.
    expect(screen.getByText("2 users · 400 lines")).toBeInTheDocument();
    // Claude Code carries both, the billed one beside the priced one.
    expect(
      screen.getByText("actual cost $12 · 1 users · 600 lines"),
    ).toBeInTheDocument();
    // the caveat is spelled out for the reader
    expect(screen.getByText(/Only Claude Code is usage-metered/)).toBeInTheDocument();
    // ...including that a day of the billed figure is a distribution, not a reading
    expect(
      screen.getByText(/Actual cost is the vendor’s monthly bill spread/),
    ).toBeInTheDocument();
  });

  it("computes the adoption funnel with data-relative stage cuts", () => {
    render(<AiCostView item="adoption-funnel" />);
    expect(screen.getByText("Adoption funnel")).toBeInTheDocument();
    expect(screen.getByText("In org")).toBeInTheDocument();
    expect(screen.getByText("Used AI (≥1 day)")).toBeInTheDocument();
    // users days = [1,3,5] → median 3 → active = {3,5} = 2; p75 = 4 → heavy = {5} = 1
    expect(screen.getByText(/Active \(≥3 days · median\)/)).toBeInTheDocument();
    expect(screen.getByText(/Heavy \(≥4 days · top quartile\)/)).toBeInTheDocument();
  });

  it("surfaces a failed per-tool breakdown instead of calling it empty", () => {
    // "No per-tool breakdown for this period" over a failed request states a
    // fact about the org that was never measured.
    mocks.tools.isError = true;
    render(<AiCostView item={null} />);
    expect(screen.getByText("Unable to load the per-tool breakdown")).toBeInTheDocument();
    expect(
      screen.queryByText(/No per-tool breakdown for this period/),
    ).not.toBeInTheDocument();
  });

  it("reads a month's seat fee and billed usage side by side, never their sum", () => {
    render(<AiCostView item={null} />);
    expect(screen.getByText("Billed by month")).toBeInTheDocument();
    const july = row("Jul 2026");
    expect(within(july).getByText("$200")).toBeInTheDocument();
    expect(within(july).getByText("$10")).toBeInTheDocument();
    // $210 is a figure the vendor never charged: the seat fee and the usage
    // billed on top of it answer different questions.
    expect(screen.queryByText("$210")).not.toBeInTheDocument();
  });

  it("says a month carries no seat fee rather than calling it $0", () => {
    // June has billed usage but no invoice priced its tier. An unpriced tier is
    // absence, and a printed $0 would claim the vendor charged nothing for it.
    render(<AiCostView item={null} />);
    const june = row("Jun 2026");
    expect(within(june).getByText("$2")).toBeInTheDocument();
    expect(within(june).getByText("—")).toBeInTheDocument();
    expect(within(june).queryByText("$0")).not.toBeInTheDocument();
  });

  it("surfaces a failed monthly request instead of calling the period uninvoiced", () => {
    mocks.monthly.isError = true;
    render(<AiCostView item={null} />);
    expect(
      screen.getByText("Unable to load the monthly billing figures"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/No invoiced months in this period/),
    ).not.toBeInTheDocument();
  });

  it("renders an honest ComingSoon for unwired pane items", () => {
    render(<AiCostView item="autofix" />);
    expect(screen.getByText(/no autofix data is collected/i)).toBeInTheDocument();
    // no fabricated KPI cards behind it
    expect(screen.queryByText("AI potential usage cost")).not.toBeInTheDocument();
  });

  it("groups cost and adoption by unit when a slice is active", () => {
    mocks.tree = person("boss", {}, [
      person("a", { division: "R&D" } as never),
      person("b", { division: "R&D" } as never),
      person("c", { division: "Sales" } as never),
      person("d", { division: "Sales" } as never),
    ]);
    act(() => portalRouter.set({ slice: "division" }));
    render(<AiCostView item="by-unit-role" />);
    expect(screen.getByText("R&D")).toBeInTheDocument();
    expect(screen.getByText("Sales")).toBeInTheDocument();
    // Both cost columns, with R&D's billed figure in the second one.
    expect(screen.getByText("Potential cost")).toBeInTheDocument();
    expect(screen.getByText("Actual cost")).toBeInTheDocument();
    expect(screen.getByText("$12")).toBeInTheDocument();
    // Sales measured zero, and a measured zero is a reading. Column 4 is
    // Actual cost: unit, people, AI users, potential, actual, lines.
    const salesCells = screen.getByText("Sales").closest("tr")?.querySelectorAll("td");
    expect(salesCells?.[4]?.textContent).toBe("$0");
  });

  it("says a unit has no billed figure rather than calling it $0", () => {
    // c and d carry no reading at all: the daily metric has no point for a day
    // nobody read. Their unit spent nothing we know of, which is not $0.
    mocks.grid.byKey = new Map([
      ...mocks.grid.byKey,
      [
        "ai.daily_approximate_extra_usage_cost",
        metric(
          "ai.daily_approximate_extra_usage_cost",
          [[pid("a"), 8], [pid("b"), 4]],
          { format: "currency", unit: "USD" } as never,
        ),
      ],
    ]);
    mocks.tree = person("boss", {}, [
      person("a", { division: "R&D" } as never),
      person("b", { division: "R&D" } as never),
      person("c", { division: "Sales" } as never),
      person("d", { division: "Sales" } as never),
    ]);
    act(() => portalRouter.set({ slice: "division" }));
    render(<AiCostView item="by-unit-role" />);
    const cells = screen.getByText("Sales").closest("tr")?.querySelectorAll("td");
    expect(cells?.[4]?.textContent).toBe("—");
  });

  it("gates on an empty scope instead of rendering zero KPIs", () => {
    mocks.members = [];
    mocks.tree = person("boss");
    render(<AiCostView item={null} />);
    expect(screen.getByText(/No people in the current scope/)).toBeInTheDocument();
    expect(screen.queryByText("AI potential usage cost")).not.toBeInTheDocument();
  });
});
