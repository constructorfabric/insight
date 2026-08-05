import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { GROUPS, KPI_ROW } from "@/lib/insight/groups";

const METRIC_KEYS = [...KPI_ROW];
const GROUP_IDS = GROUPS.map((def) => def.id);

const kpiState = {
  isPending: false,
  isFetching: false,
  isError: false,
  refetch: vi.fn(),
};
let personData: { display_name?: string } | undefined;
let personError: unknown;
let tilesReturn: Array<{ key: string }> = [];
let attentionPerGroup: Array<{ key: string }> = [];
let omitGroupId: string | null = null;

const personRefetch = vi.fn();

vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({
    data: personData,
    error: personError,
    isError: personError !== undefined,
    refetch: personRefetch,
  }),
}));

vi.mock("@/hooks/use-period", () => ({
  usePeriod: () => ({
    period: "month",
    dateRange: { from: "2026-01-01", to: "2026-01-31" },
    setPeriod: vi.fn(),
  }),
}));

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all" }),
}));

vi.mock("@/lib/insight/kpi-row", () => ({
  metricKpiTiles: () => tilesReturn,
}));

vi.mock("@/lib/insight/attention", () => ({
  metricAttentionItems: () => attentionPerGroup,
}));

vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: () => ({
    byKey: new Map(),
    previousByKey: null,
    ...kpiState,
  }),
  useMetricCollectionSet: () => {
    const map = new Map();
    for (const id of GROUP_IDS) {
      if (id === omitGroupId) continue;
      map.set(id, {
        byKey: new Map(),
        previousByKey: null,
        isPending: false,
        isFetching: false,
        isError: false,
        refetch: vi.fn(),
      });
    }
    return map;
  },
  collectionSetPending: (set: Map<string, { isPending: boolean }>) =>
    [...set.values()].some((result) => result.isPending),
}));

vi.mock("@/components/widgets/dashboard/dashboard-header", () => ({
  DashboardHeader: ({ title }: { title: string }) => (
    <div data-testid="header">{title}</div>
  ),
}));

vi.mock("@/components/widgets/dashboard/ic-needs-attention", () => ({
  IcNeedsAttention: ({ items }: { items: unknown[] }) => (
    <div data-testid="attention">{items.length}</div>
  ),
}));

vi.mock("@/components/widgets/dashboard/kpi-tile", () => ({
  KpiTile: ({ tile }: { tile: { key: string } }) => (
    <div data-testid="kpi-tile">{tile.key}</div>
  ),
  KpiTilePlaceholder: () => <div data-testid="kpi-placeholder" />,
}));

vi.mock("@/components/widgets/coming-soon", () => ({
  ComingSoon: ({ onRetry, label }: { onRetry?: () => void; label?: string }) => (
    <button data-testid="kpi-error" onClick={onRetry} disabled={!onRetry}>
      {label ?? "retry"}
    </button>
  ),
}));

vi.mock("@/components/widgets/metric-views/metric-group-card", () => ({
  MetricGroupCard: ({
    def,
    onOpen,
  }: {
    def: { id: string };
    onOpen: () => void;
  }) => (
    <button data-testid="metric-card" data-group={def.id} onClick={onOpen}>
      {def.id}
    </button>
  ),
}));

vi.mock("@/components/widgets/dashboard/group-drilldown-sheet", () => ({
  GroupDrilldownSheet: ({
    def,
    open,
  }: {
    def: { id: string };
    open: boolean;
  }) => (
    <div
      data-testid="drilldown"
      data-group={def.id}
      data-open={open ? "true" : "false"}
    />
  ),
}));

import { DashboardScreen } from "./dashboard";

beforeEach(() => {
  kpiState.isPending = false;
  kpiState.isFetching = false;
  kpiState.isError = false;
  kpiState.refetch = vi.fn();
  tilesReturn = [];
  attentionPerGroup = [];
  omitGroupId = null;
  personData = undefined;
  personError = undefined;
  personRefetch.mockReset();
});

describe("DashboardScreen identity failures", () => {
  const PERSON_ID = "019e2805-0000-7000-8000-00000000a11c";

  it("says the person is unavailable instead of painting a nameless dashboard", async () => {
    // A valid UUID outside the viewer's visible set is a 404: without a person
    // there is no name and the metrics below are unauthorized anyway.
    const { IdentityApiError } = await import("@/api/identity-client");
    personError = new IdentityApiError(404, {});
    tilesReturn = METRIC_KEYS.map((key) => ({ key }));

    render(<DashboardScreen personId={PERSON_ID} />);

    const state = screen.getByTestId("kpi-error");
    expect(state).toHaveTextContent("This person is not available");
    // Nothing to retry — the answer will not change.
    expect(state).toBeDisabled();
    expect(screen.queryByTestId("kpi-tile")).not.toBeInTheDocument();
  });

  it("offers a retry when identity itself failed, not the lookup", async () => {
    const { IdentityApiError } = await import("@/api/identity-client");
    personError = new IdentityApiError(500, {});

    render(<DashboardScreen personId={PERSON_ID} />);

    const state = screen.getByTestId("kpi-error");
    expect(state).toHaveTextContent("Unable to load this person");
    await userEvent.click(state);
    expect(personRefetch).toHaveBeenCalled();
  });
});

describe("DashboardScreen", () => {
  it("renders a KPI tile, a card per group, and the attention section", () => {
    tilesReturn = METRIC_KEYS.map((key) => ({ key }));
    attentionPerGroup = [{ key: "k" }];

    personData = { display_name: "Me Person" };

    render(<DashboardScreen personId="me@x.io" />);

    // The header shows the resolved name, never the raw id/email.
    expect(screen.getByTestId("header")).toHaveTextContent("Me Person");
    expect(screen.getByTestId("header")).not.toHaveTextContent("me@x.io");
    expect(screen.getAllByTestId("kpi-tile")).toHaveLength(METRIC_KEYS.length);
    expect(screen.getAllByTestId("metric-card")).toHaveLength(GROUP_IDS.length);
    expect(screen.getAllByTestId("drilldown")).toHaveLength(GROUP_IDS.length);
    expect(screen.getByTestId("attention")).toBeInTheDocument();
    expect(
      screen
        .getAllByTestId("drilldown")
        .every((el) => el.dataset.open === "false")
    ).toBe(true);
  });

  it("shows a retryable error tile per KPI when the collection errored", async () => {
    kpiState.isError = true;

    render(<DashboardScreen personId="me@x.io" />);

    const errors = screen.getAllByTestId("kpi-error");
    expect(errors).toHaveLength(METRIC_KEYS.length);
    await userEvent.click(errors[0]!);
    expect(kpiState.refetch).toHaveBeenCalled();
  });

  it("shows the single page spinner while the collection is pending", () => {
    kpiState.isPending = true;

    render(<DashboardScreen personId="me@x.io" />);

    // One loading state for the whole dashboard — no per-widget loaders and
    // no partially painted content.
    expect(screen.getByRole("status", { name: "Loading" })).toBeInTheDocument();
    expect(screen.queryByTestId("kpi-tile")).not.toBeInTheDocument();
    expect(screen.queryByTestId("attention")).not.toBeInTheDocument();
  });

  it("shows a placeholder per KPI when settled with no data", () => {
    render(<DashboardScreen personId="me@x.io" />);

    expect(screen.getAllByTestId("kpi-placeholder")).toHaveLength(
      METRIC_KEYS.length
    );
  });

  it("renders no card for a group with no query result", () => {
    omitGroupId = GROUP_IDS[0]!;

    render(<DashboardScreen personId="me@x.io" />);

    expect(screen.getAllByTestId("metric-card")).toHaveLength(
      GROUP_IDS.length - 1
    );
  });

  it("opens the drilldown for the group whose card is clicked", async () => {
    render(<DashboardScreen personId="me@x.io" />);

    const firstGroup = GROUP_IDS[0]!;
    await userEvent.click(screen.getByRole("button", { name: firstGroup }));

    const sheet = screen
      .getAllByTestId("drilldown")
      .find((el) => el.dataset.group === firstGroup);
    expect(sheet?.dataset.open).toBe("true");
  });

  it("closes any open drilldown when the viewed person changes", async () => {
    const { rerender } = render(<DashboardScreen personId="me@x.io" />);

    await userEvent.click(screen.getByRole("button", { name: GROUP_IDS[0]! }));
    expect(
      screen
        .getAllByTestId("drilldown")
        .some((el) => el.dataset.open === "true")
    ).toBe(true);

    rerender(<DashboardScreen personId="other@x.io" />);
    expect(
      screen
        .getAllByTestId("drilldown")
        .every((el) => el.dataset.open === "false")
    ).toBe(true);
  });
});
