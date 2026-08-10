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
}));

vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: () => mocks.collection,
  useMetricCollectionSet: () => mocks.set,
  collectionSetPending: (set: Map<string, { isPending: boolean }>) =>
    [...set.values()].some((r) => r.isPending),
}));
vi.mock("@/lib/portal/use-person-cohort", () => ({
  usePersonCohort: () => mocks.cohort,
}));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({ period: "week", dateRange: { from: "2026-07-20", to: "2026-07-26" } }),
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
  seedSet();
});

describe("MetricGroupsView", () => {
  it("renders an honest note when no group is in the semantic layer", () => {
    render(<MetricGroupsView personId="p@x" groupIds={[]} />);
    expect(screen.getByText(/Not in the semantic layer yet/)).toBeInTheDocument();
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
