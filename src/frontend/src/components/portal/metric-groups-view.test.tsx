// @vitest-environment jsdom
vi.mock("@/lib/portal/use-cohort-label", () => ({
  useCohortLabel: () => "team",
}));
/**
 * MetricGroupsView routing/gating semantics: honest empty + error + loading
 * states, KPI-row gating via showKpis, section-card wiring and the
 * inline-select vs modal-drilldown split. The v2 leaf widgets (KpiTile,
 * MetricGroupCard, drilldown) predate this PR and are stubbed; assertions
 * target THIS view's own decisions.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import { previousPeriodRange } from "@/api/period-to-date-range";
import type { GroupId } from "@/lib/insight/groups";
import { normalizeMetricResults } from "@/lib/metrics/collection";

const mocks = vi.hoisted(() => ({
  collection: {
    byKey: new Map(),
    previousByKey: new Map(),
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
  },
  set: new Map<string, { byKey: Map<string, never>; isPending: boolean; isError: boolean; refetch: () => void }>(),
  cohort: [] as string[],
  period: "week" as const,
  range: { from: "2026-07-20", to: "2026-07-26" },
  // Every `useMetricCollectionSet` call, so a test can assert WHICH windows the
  // view asked for — the mock returns one result set regardless of arguments.
  setCalls: [] as Array<{
    collections: unknown;
    entity: { type: string; ids?: string[] };
    range: { from: string; to: string };
    compareTo?: { from: string; to: string };
  }>,
}));

// usePersonSectionStandings now reads source availability from the tenant's
// definition listing rather than inferring it from an empty comparison pool.
vi.mock("@/queries/metric-definitions", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  useMetricDefinitionsResponse: () => ({
    data: { metrics: [] },
    isPending: false,
    isError: false,
  }),
}));
vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: () => mocks.collection,
  useMetricCollectionSet: (
    collections: unknown,
    entity: { type: string; ids?: string[] },
    range: { from: string; to: string },
    compareTo?: { from: string; to: string }
  ) => {
    mocks.setCalls.push({ collections, entity, range, compareTo });
    return mocks.set;
  },
  collectionSetPending: (set: Map<string, { isPending: boolean }>) =>
    [...set.values()].some((r) => r.isPending),
}));
vi.mock("@/lib/portal/use-person-cohort", () => ({
  usePersonCohort: () => mocks.cohort,
}));
// This view only mounts on "At a glance", so no section is ever open under it;
// `use-person-sections.test.tsx` covers the window scope on both routes.
vi.mock("@/lib/portal/portal-nav", () => ({
  usePortalItem: () => null,
}));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({ period: mocks.period, dateRange: mocks.range }),
}));
vi.mock("@/hooks/use-settings", () => ({ useSettings: () => ({ focusMode: false }) }));
vi.mock("@/components/widgets/dashboard/kpi-tile", () => ({
  KpiTile: ({
    tile,
    onOpenGroup,
  }: {
    tile: { key: string; groupId: string | null };
    onOpenGroup?: (id: string) => void;
  }) => (
    <button
      data-testid="kpi-tile"
      onClick={() => tile.groupId && onOpenGroup?.(tile.groupId)}
    >
      {tile.key}
    </button>
  ),
  KpiTilePlaceholder: () => <div data-testid="kpi-placeholder" />,
}));
vi.mock("@/components/widgets/dashboard/ic-needs-attention", () => ({
  IcNeedsAttention: () => <div data-testid="needs-attention" />,
}));
vi.mock("@/components/widgets/metric-views/metric-group-card", () => ({
  MetricGroupCard: ({ def, onOpen }: { def: { id: string; title: string }; onOpen: () => void }) => (
    <button data-testid={`group-card-${def.id}`} onClick={onOpen}>
      {def.title}
    </button>
  ),
}));
vi.mock("@/components/widgets/dashboard/group-drilldown-sheet", () => ({
  GroupDrilldownSheet: ({ open, def }: { open: boolean; def: { id: string } }) =>
    open ? <div data-testid={`drilldown-${def.id}`} /> : null,
}));

import { MetricGroupsView } from "./metric-groups-view";

const GROUPS: readonly GroupId[] = ["git_output", "collaboration"];

/** One metric of the group, with a real value — a group with no values at all
 *  is collapsed into a line now, so a card test has to give it something. */
function metric(key: string): MetricResult {
  return {
    metric_key: key,
    label: key,
    unit: null,
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
    views: [{ view: "period", values: [{ entity_id: "p@x", value: 3 }] }],
  } as MetricResult;
}

const GROUP_METRIC: Record<string, string> = {
  git_output: "git.commits",
  collaboration: "collab.messages_sent",
};

function seedSet(
  over: Partial<{ isPending: boolean; isError: boolean }> = {},
  { withData = true } = {},
) {
  mocks.set = new Map(
    GROUPS.map((id) => [
      id as string,
      {
        byKey: withData
          ? normalizeMetricResults([metric(GROUP_METRIC[id]!)])
          : new Map<string, never>(),
        isPending: false,
        isError: false,
        refetch: vi.fn() as () => void,
        ...over,
      },
    ]),
  ) as typeof mocks.set;
}

beforeEach(() => {
  mocks.collection.isPending = false;
  mocks.collection.isError = false;
  mocks.cohort = [];
  mocks.setCalls.length = 0;
  seedSet();
});

describe("MetricGroupsView", () => {
  it("renders an honest note when no group is in the semantic layer", () => {
    render(<MetricGroupsView personId="p@x" groupIds={[]} />);
    expect(screen.getByText(/Not available yet for this direction/)).toBeInTheDocument();
  });

  it("spins while any group collection is pending", () => {
    seedSet({ isPending: true });
    const { container } = render(<MetricGroupsView personId="p@x" groupIds={GROUPS} />);
    expect(screen.queryByTestId("group-card-git_output")).not.toBeInTheDocument();
    expect(container.querySelector(".animate-spin")).not.toBeNull();
  });

  it("surfaces a group failure as a retryable error, not empty cards", async () => {
    seedSet({ isError: true });
    render(<MetricGroupsView personId="p@x" groupIds={GROUPS} />);
    expect(screen.queryByTestId("group-card-git_output")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(mocks.set.get("git_output")!.refetch).toHaveBeenCalled();
  });

  it("renders NO section cards — the nav carries the sections", () => {
    // The page is an overview and every section has its own screen, listed in
    // the nav to the left with a standing mark on it. Cards here restated that
    // list a second time and answered a question the nav already answers.
    render(<MetricGroupsView personId="p@x" groupIds={GROUPS} />);
    expect(screen.queryByTestId("group-card-git_output")).not.toBeInTheDocument();
    expect(screen.queryByTestId("group-card-collaboration")).not.toBeInTheDocument();
    // KPI row is opt-in and off here
    expect(screen.queryByTestId("needs-attention")).not.toBeInTheDocument();
  });

  it("shows the KPI row + needs-attention only with showKpis", () => {
    render(<MetricGroupsView personId="p@x" groupIds={GROUPS} showKpis />);
    expect(screen.getByText("At a glance")).toBeInTheDocument();
    expect(screen.getByTestId("needs-attention")).toBeInTheDocument();
  });

  it("asks for the sections over one window pair, not a shifted second request", () => {
    render(<MetricGroupsView personId="p@x" groupIds={GROUPS} showKpis />);
    // Attention compares against the previous period. Asking for it over a
    // SHIFTED range is the duplicate round trip this view stopped making
    // (#2651), so every request runs over the period on screen.
    for (const call of mocks.setCalls) {
      expect(call.range, `range for ${JSON.stringify(call.entity)}`).toEqual(
        mocks.range,
      );
    }
    // The page and the nav mark both ask; identical arguments is what makes
    // that one query key and one round trip, so the distinct signature — not
    // the call count — is the thing under test.
    const sections = mocks.setCalls.filter((c) => (c.entity.ids?.length ?? 0) > 0);
    const signatures = new Set(sections.map((c) => JSON.stringify(c)));
    expect(sections.length).toBeGreaterThan(1);
    expect(signatures.size).toBe(1);
    expect(sections[0]!.compareTo).toEqual(
      previousPeriodRange(mocks.range, mocks.period),
    );
  });

  it("asks for the slice cohort under one signature too", () => {
    mocks.cohort = ["a@x", "b@x"];
    render(<MetricGroupsView personId="p@x" groupIds={GROUPS} showKpis />);
    // The same trap one hook over: the page and the nav mark each named their
    // own group list for the cohort, so a narrower one would split it into a
    // second key and a second request per section.
    const cohort = mocks.setCalls.filter(
      (c) => c.entity.ids?.join() === mocks.cohort.join(),
    );
    expect(cohort.length).toBeGreaterThan(1);
    expect(new Set(cohort.map((c) => JSON.stringify(c))).size).toBe(1);
  });

  it("routes a KPI tile through onSelectGroup instead of the modal when provided", async () => {
    // The row renders metrics the person is OBSERVED on, so the fixture needs a
    // peer row — an unmeasured metric gets no tile and nothing to click.
    mocks.collection.byKey = normalizeMetricResults([
      {
        ...metric("git.commits"),
        views: [
          { view: "period", values: [{ entity_id: "p@x", value: 3 }] },
          {
            view: "peer",
            values: [
              {
                entity_id: "p@x",
                target_value: 3,
                p25: 1,
                median: 5,
                p75: 9,
                min: 0,
                max: 12,
                n: 8,
              },
            ],
          },
        ],
      } as MetricResult,
    ]);
    // The tiles and the attention rows are the openers now that the cards are
    // gone; inline selection is what the portal passes, so the section opens
    // in place rather than in a sheet over it.
    const onSelect = vi.fn();
    render(
      <MetricGroupsView
        personId="p@x"
        groupIds={GROUPS}
        showKpis
        onSelectGroup={onSelect}
      />,
    );
    await userEvent.click(screen.getAllByTestId("kpi-tile")[0]!);
    expect(onSelect).toHaveBeenCalled();
    expect(screen.queryByTestId("drilldown-git_output")).not.toBeInTheDocument();
  });
});
