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

import type { GroupId } from "@/lib/insight/groups";

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
  usePortalPeriod: () => ({ period: "week", dateRange: { start: "2026-07-20", end: "2026-07-26" } }),
}));
vi.mock("@/hooks/use-settings", () => ({ useSettings: () => ({ focusMode: false }) }));
vi.mock("@/components/widgets/dashboard/kpi-tile", () => ({
  KpiTile: ({ tile }: { tile: { key: string } }) => <div data-testid="kpi-tile">{tile.key}</div>,
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

function seedSet(over: Partial<{ isPending: boolean; isError: boolean }> = {}) {
  mocks.set = new Map(
    GROUPS.map((id) => [
      id as string,
      { byKey: new Map<string, never>(), isPending: false, isError: false, refetch: vi.fn() as () => void, ...over },
    ]),
  );
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

  it("renders a section card per requested group", () => {
    render(<MetricGroupsView personId="p@x" groupIds={GROUPS} />);
    expect(screen.getByTestId("group-card-git_output")).toBeInTheDocument();
    expect(screen.getByTestId("group-card-collaboration")).toBeInTheDocument();
    // KPI row is opt-in and off here
    expect(screen.queryByTestId("needs-attention")).not.toBeInTheDocument();
  });

  it("shows the KPI row + needs-attention only with showKpis", () => {
    render(<MetricGroupsView personId="p@x" groupIds={GROUPS} showKpis />);
    expect(screen.getByText("At a glance")).toBeInTheDocument();
    expect(screen.getByTestId("needs-attention")).toBeInTheDocument();
  });

  it("routes a card through onSelectGroup instead of the modal when provided", async () => {
    const onSelect = vi.fn();
    render(<MetricGroupsView personId="p@x" groupIds={GROUPS} onSelectGroup={onSelect} />);
    await userEvent.click(screen.getByTestId("group-card-git_output"));
    expect(onSelect).toHaveBeenCalledWith("git_output");
    expect(screen.queryByTestId("drilldown-git_output")).not.toBeInTheDocument();
  });

  it("opens the drilldown modal when no inline selector is wired", async () => {
    render(<MetricGroupsView personId="p@x" groupIds={GROUPS} />);
    await userEvent.click(screen.getByTestId("group-card-git_output"));
    expect(screen.getByTestId("drilldown-git_output")).toBeInTheDocument();
  });
});
