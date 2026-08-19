// @vitest-environment jsdom
/**
 * Shell + router semantics: which view each zone/item/lens resolves to, the
 * manager-vs-IC landing decision, the route→scope one-shot sync and the
 * global slice control. Heavy leaf views are stubbed — their own behavior is
 * covered in their dedicated test files; here we assert the ROUTING.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { resetRouteScopeSync } from "@/lib/portal/route-scope-sync";
import { pid } from "@/test/identity";
import { portalRouter } from "@/test/portal-router";

import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  personId: null as string | null,
  isManager: true,
  isPending: false,
  zone: { activeZone: "overview", activePerson: "" },
}));

vi.mock("@/auth", () => ({
  useViewer: () => ({ email: "boss@x", personId: mocks.personId }),
}));
vi.mock("@/queries/identity-me", () => ({
  useIsAdmin: () => ({ isAdmin: false, isPending: false }),
}));
vi.mock("@/lib/portal/use-viewer-is-manager", () => ({
  useViewerIsManager: () => ({ isManager: mocks.isManager, isPending: mocks.isPending }),
}));
vi.mock("@/lib/portal/use-active-zone", () => ({
  useActiveZone: () => mocks.zone,
}));
vi.mock("@/lib/portal/use-cohort-label", () => ({
  useCohortLabel: () => "team",
}));
vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({ data: undefined, isPending: false, isLoading: false, isError: false, refetch: vi.fn() }),
}));

// Leaf views become labelled placeholders so routing is observable.
vi.mock("@/components/portal/domain-lens-view", () => ({
  DomainLensView: ({ config }: { config: { title: string } }) => (
    <div data-testid="domain-lens">{config.title}</div>
  ),
  Pending: ({ label }: { label: string }) => <div data-testid="pending">{label}</div>,
}));
vi.mock("@/components/portal/ai-cost-view", () => ({
  AiCostView: () => <div data-testid="ai-cost" />,
}));
vi.mock("@/components/portal/manage-view", () => ({
  ManageView: () => <div data-testid="manage" />,
}));
vi.mock("@/components/portal/team-state-view", () => ({
  TeamStateView: () => <div data-testid="team-state" />,
}));
vi.mock("@/components/portal/employees-view", () => ({
  EmployeesView: () => <div data-testid="employees" />,
}));
vi.mock("@/components/portal/metric-groups-view", () => ({
  MetricGroupsView: ({ personId }: { personId: string }) => (
    <div data-testid="metric-groups">{personId}</div>
  ),
}));
vi.mock("@/components/portal/single-group-view", () => ({
  SingleGroupView: ({ groupId }: { groupId: string }) => (
    <div data-testid="single-group">{groupId}</div>
  ),
}));
vi.mock("@/components/portal/person-header", () => ({
  PersonHeader: ({ person }: { person: string }) => (
    <div data-testid="person-header">{person}</div>
  ),
}));
vi.mock("@/components/portal/lens-rail", () => ({ LensRail: () => <div /> }));
vi.mock("@/components/portal/context-pane", () => ({ ContextPane: () => <div /> }));
vi.mock("@/components/portal/portal-topbar", () => ({ PortalTopBar: () => <div /> }));
vi.mock("@/components/mock-banner", () => ({ MockBanner: () => <div /> }));
vi.mock("@/components/view-as-banner", () => ({
  ViewAsBanner: () => <div data-testid="view-as-banner" />,
}));

import {
  usePortalScope,
  usePortalSlice,
  usePortalZone,
} from "@/lib/portal/portal-nav";
import { renderHook } from "@testing-library/react";

import { DirectionView } from "./direction-view";
import { OverviewView } from "./overview-view";
import { PeopleView } from "./people-view";
import { PersonView } from "./person-view";
import { PortalLayout } from "./portal-layout";
import { SliceSelect } from "./slice-select";
import { ZoneContent } from "./zone-content";

// SidebarProvider (inside PortalLayout) reads a media query; jsdom has none.
beforeEach(() => {
  window.matchMedia ??= ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  mocks.isManager = true;
  mocks.isPending = false;
  mocks.personId = pid("boss");
  resetRouteScopeSync();
  mocks.zone = { activeZone: "overview", activePerson: pid("boss") };
  act(() => {
    portalRouter.set({ zone: undefined });
    portalRouter.set({ item: undefined });
    portalRouter.set({ dir: "dev" });
    portalRouter.set({ lens: "Delivery" });
    portalRouter.set({ slice: undefined });
    portalRouter.set({ scope: undefined, direct: false });
  });
});

describe("ZoneContent routing", () => {
  const cases: Array<[string, string]> = [
    ["person", "person-header"],
    ["overview", "domain-lens"],
    ["directions", "domain-lens"],
    ["aicost", "ai-cost"],
    ["people", "team-state"],
    ["manage", "manage"],
  ];
  it.each(cases)("zone %s renders its view", (zone, testid) => {
    mocks.zone = { activeZone: zone, activePerson: pid("boss") };
    render(<ZoneContent />);
    expect(screen.getByTestId(testid)).toBeInTheDocument();
  });

  it("scorecard renders an honest scaffold, not a fake dashboard", () => {
    mocks.zone = { activeZone: "scorecard", activePerson: pid("boss") };
    render(<ZoneContent />);
    expect(screen.getByText("Scorecard")).toBeInTheDocument();
    expect(screen.getByText(/org snapshots/)).toBeInTheDocument();
  });
});

describe("DirectionView", () => {
  it("routes a configured lens to DomainLensView", () => {
    render(<DirectionView dir="dev" lens="Overview" />);
    expect(screen.getByTestId("domain-lens")).toBeInTheDocument();
  });

  it("renders the roadmap note for a ComingSoon lens", () => {
    render(<DirectionView dir="dev" lens="Repositories" />);
    expect(screen.getByTestId("pending").textContent).toMatch(/Repository-level rollups/);
  });

  it("names the direction in the unknown-lens note", () => {
    render(<DirectionView dir="dev" lens="Nope" />);
    expect(screen.getByTestId("pending").textContent).toMatch(
      /“Nope” isn't a metric family in Development yet/,
    );
  });

  it("asks for a direction when the pane collapsed the selection", () => {
    // Collapsing a direction clears `dir`; blaming the lens for missing from
    // a direction the reader never picked reads like a bug.
    render(<DirectionView dir="" lens="Delivery" />);
    expect(screen.getByTestId("pending").textContent).toMatch(/Pick a direction/);
  });
});

describe("OverviewView", () => {
  it("defaults a null item to At a glance", () => {
    render(<OverviewView item={null} />);
    expect(screen.getByTestId("domain-lens")).toHaveTextContent("Overview");
  });

  it("resolves a named item through the registry", () => {
    render(<OverviewView item="contribution" />);
    expect(screen.getByTestId("domain-lens")).toHaveTextContent(
      "Overview · Contribution breakdown",
    );
  });

  it("renders Pending for an unknown item instead of crashing", () => {
    render(<OverviewView item="nope" />);
    expect(screen.getByTestId("pending").textContent).toMatch(/isn't an Overview view yet/);
  });
});

describe("PersonView", () => {
  it("defaults to the at-a-glance dashboard with the person's header", () => {
    render(<PersonView person={pid("a")} />);
    expect(screen.getByTestId("person-header")).toHaveTextContent(pid("a"));
    expect(screen.getByTestId("metric-groups")).toHaveTextContent(pid("a"));
  });

  it("expands a selected metric group inline", () => {
    act(() => portalRouter.set({ item: "git_output" }));
    render(<PersonView person={pid("a")} />);
    expect(screen.getByTestId("single-group")).toHaveTextContent("git_output");
    expect(screen.queryByTestId("metric-groups")).not.toBeInTheDocument();
  });
});

describe("PeopleView", () => {
  it("routes items: roster (default), employees, median-by-role scaffold", () => {
    const { rerender } = render(<PeopleView person={pid("p1")} item={null} />);
    expect(screen.getByTestId("team-state")).toBeInTheDocument();
    rerender(<PeopleView person={pid("p1")} item="employees" />);
    expect(screen.getByTestId("employees")).toBeInTheDocument();
    rerender(<PeopleView person={pid("p1")} item="median-by-role" />);
    expect(screen.getByText(/Medians by role/)).toBeInTheDocument();
  });

  it("syncs the route person into the org scope ONCE, then defers to the user", () => {
    const scope = renderHook(() => usePortalScope());
    const first = render(<PeopleView person={pid("p2")} item={null} />);
    expect(scope.result.current.root).toBe(pid("p2"));
    // The user re-picks a scope from the topbar…
    act(() => portalRouter.set({ scope: pid("other") }));
    // …and leaving the zone and coming back must NOT revert it. Unmounted for
    // real, so this is the remount case and not two live instances.
    first.unmount();
    render(<PeopleView person={pid("p2")} item={null} />);
    expect(scope.result.current.root).toBe(pid("other"));
  });
});

describe("PortalLayout landing", () => {
  it("pins a manager's landing zone to Overview once resolved", () => {
    const zone = renderHook(() => usePortalZone());
    render(<PortalLayout />);
    expect(zone.result.current).toBe("overview");
  });

  it("leaves an IC route-driven (their own Person page)", () => {
    mocks.isManager = false;
    const zone = renderHook(() => usePortalZone());
    render(<PortalLayout />);
    expect(zone.result.current).toBeNull();
  });

  it("never overrides a zone the manager picked while their role resolved", () => {
    act(() => portalRouter.set({ zone: "directions" }));
    const zone = renderHook(() => usePortalZone());
    render(<PortalLayout />);
    expect(zone.result.current).toBe("directions");
  });

  it("resets an IC stranded on an org zone (hidden for them) to route-driven", () => {
    mocks.isManager = false;
    act(() => portalRouter.set({ zone: "overview" }));
    const zone = renderHook(() => usePortalZone());
    render(<PortalLayout />);
    expect(zone.result.current).toBeNull();
  });
});

describe("SliceSelect", () => {
  it("shows the active dimension and writes the store on change", async () => {
    const slice = renderHook(() => usePortalSlice());
    render(<SliceSelect dims={[{ key: "division", label: "Division" }]} />);
    // "Slice:" is a separate, md-only span now, so match the value itself.
    expect(screen.getByRole("combobox", { name: "Cohort" })).toHaveTextContent(
      "Team (all)",
    );

    await userEvent.click(screen.getByRole("combobox", { name: "Cohort" }));
    await userEvent.click(await screen.findByRole("option", { name: "Division" }));
    expect(slice.result.current).toBe("division");
  });

  it("maps the team sentinel back to an empty slice", async () => {
    act(() => portalRouter.set({ slice: "division" }));
    render(<SliceSelect dims={[{ key: "division", label: "Division" }]} />);
    const slice = renderHook(() => usePortalSlice());
    await userEvent.click(screen.getByRole("combobox", { name: "Cohort" }));
    await userEvent.click(await screen.findByRole("option", { name: "Team (all)" }));
    expect(slice.result.current).toBe("");
  });
});
