// @vitest-environment jsdom
/**
 * Team-state dashboard semantics: honest KPI roll-ups (totals for counters,
 * medians for ratios), all-empty columns dropped instead of zero-painted,
 * attention wired to the roster, and the org-scope gate in front of it all.
 * Data arrives via the same stubbed query boundary as the real view.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import type { PeopleListItem } from "@/api/identity-client";
import { identityPerson, peopleFromIdentityTree, pid } from "@/test/identity";
import type { IdentityPerson } from "@/types/insight";

const mocks = vi.hoisted(() => ({
  personId: null as string | null,
  tree: undefined as IdentityPerson | undefined,
  roster: [] as PeopleListItem[],
  grid: {
    byKey: new Map<string, NormalizedMetricResult>(),
    previousByKey: new Map<string, NormalizedMetricResult>(),
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
    roster: mocks.roster,
    isPending: false,
    isError: false,
    retry: () => {},
  }),
}));
vi.mock("@/queries/member-grid", () => ({ useMemberGridData: () => mocks.grid }));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({ period: "week", dateRange: { from: "2026-07-20", to: "2026-07-26" } }),
}));


import { TeamStateView } from "./team-state-view";

const person = (label: string, subs: IdentityPerson[] = []): IdentityPerson =>
  identityPerson(label, {}, subs);


function metric(
  key: string,
  period: Array<[string, number | null]>,
  over: Partial<NormalizedMetricResult> = {},
): NormalizedMetricResult {
  return {
    metric_key: key,
    label: over.label ?? key,
    unit: null,
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    period: { view: "period", values: period.map(([entity_id, value]) => ({ entity_id, value })) },
    peer: { view: "peer", values: period.map(([entity_id, value]) => ({ entity_id, target_value: value })) },
    ...over,
  } as unknown as NormalizedMetricResult;
}

// Roster entity ids: person UUIDs, the same key the metric grid returns.
const LABELS = ["a", "b", "c", "d"];
const MEMBER_LABELS = ["boss", ...LABELS];
const MEMBER_IDS = MEMBER_LABELS.map(pid);

beforeEach(() => {
  mocks.personId = pid("boss");
  mocks.tree = person("boss", LABELS.map((l) => person(l)));
  mocks.roster = peopleFromIdentityTree(mocks.tree);
  mocks.grid.isPending = false;
  mocks.grid.isError = false;
  // git.commits is a real headline key (GROUPS card.preview) — the view
  // only renders columns from that set.
  mocks.grid.byKey = new Map([
    ["git.commits", metric("git.commits", [[pid("boss"), 50], [pid("a"), 10], [pid("b"), 20], [pid("c"), 30], [pid("d"), 40]], { label: "Commits" })],
    // a ratio metric: must roll up as MEDIAN, not a summed percentage
    ["collab.focus_time_pct", metric("collab.focus_time_pct", [[pid("boss"), 80], [pid("a"), 40], [pid("b"), 50], [pid("c"), 60], [pid("d"), 70]], {
      computation: "avg",
      label: "Focus Time",
      format: "percent",
    } as never)],
    // ingested nowhere → its column must disappear, not paint zeros
    ["tasks.closed", metric("tasks.closed", MEMBER_IDS.map((id) => [id, null]), { label: "Tasks closed" })],
  ]);
  mocks.grid.previousByKey = new Map();
  act(() => {
    portalRouter.set({ slice: undefined });
    portalRouter.set({ scope: undefined, direct: false });
  });
});

describe("TeamStateView", () => {
  it("renders the scope header and every member row", () => {
    render(<TeamStateView />);
    expect(screen.getByText("boss's team")).toBeInTheDocument();
    expect(screen.getByText(/5 people · state & attention/)).toBeInTheDocument();
    for (const label of MEMBER_LABELS) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  it("sums counters into a team total and medians ratios — never the reverse", () => {
    render(<TeamStateView />);
    expect(screen.getByText("150")).toBeInTheDocument();
    expect(screen.getAllByText("team total").length).toBeGreaterThan(0);
    // median of 40,50,60,70,80 = 60%, NOT the 300% a sum would fabricate
    expect(screen.getAllByText("60%").length).toBeGreaterThan(0);
    expect(screen.getAllByText("team median").length).toBeGreaterThan(0);
  });

  it("drops a column that has no data for anyone (honest, not zero-filled)", () => {
    render(<TeamStateView />);
    expect(screen.queryByText("Tasks closed")).not.toBeInTheDocument();
  });

  it("keeps the steady attention note when nobody diverges", () => {
    render(<TeamStateView />);
    expect(screen.getByText("All 5 people are in their usual range this period.")).toBeInTheDocument();
    expect(screen.getByText("Nothing stands out this period.")).toBeInTheDocument();
  });

  it("includes the manager in needs attention", () => {
    mocks.grid.byKey.set(
      "git.commits",
      metric("git.commits", [[pid("boss"), 0], [pid("a"), 10], [pid("b"), 20], [pid("c"), 30], [pid("d"), 40]], { label: "Commits" }),
    );

    render(<TeamStateView />);

    expect(screen.getByText(/1 of 5 people stands out this period/)).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: /Commits/, expanded: false }),
    );
    expect(screen.getAllByText("boss")).toHaveLength(2);
  });

  it("lists the roster by name, like the org tree beside it", () => {
    // d and boss trail both metrics, so a standing order would float them up.
    mocks.grid.byKey = new Map([
      ["git.commits", metric("git.commits", [[pid("boss"), 20], [pid("a"), 50], [pid("b"), 40], [pid("c"), 30], [pid("d"), 10]], { label: "Commits" })],
      ["collab.focus_time_pct", metric("collab.focus_time_pct", [[pid("boss"), 50], [pid("a"), 80], [pid("b"), 70], [pid("c"), 60], [pid("d"), 40]], {
        computation: "avg",
        label: "Focus Time",
        format: "percent",
      } as never)],
    ]);

    render(<TeamStateView />);

    expect(
      screen.getAllByRole("rowheader").map((th) => th.textContent),
    ).toEqual(["a", "b", "boss", "c", "d"]);
  });

  it("shows a manager with no reports as a one-person scope", () => {
    mocks.tree = person("boss");
    mocks.roster = peopleFromIdentityTree(mocks.tree);
    render(<TeamStateView />);
    expect(screen.getByText(/1 people · state & attention/)).toBeInTheDocument();
    expect(screen.getByText("boss")).toBeInTheDocument();
  });
});
