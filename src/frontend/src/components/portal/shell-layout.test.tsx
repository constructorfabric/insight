// @vitest-environment jsdom
/**
 * Mobile shell semantics. Two fixed sidebars (56px rail + 256px pane) left a
 * 375px phone ~60px of content, so below the breakpoint the rail hides and the
 * pane becomes the single off-canvas drawer. These tests pin the parts that
 * make that usable: the rail really disappears, the drawer carries the zones
 * (collapsed, so the zone's own sections stay above the fold) and the settings
 * menu, a zone pick keeps the drawer open while a section pick closes it, and
 * NOTHING of this leaks into the desktop layout.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  layout: "phone" as "phone" | "narrow" | "wide",
  zone: { activeZone: "overview", activePerson: "boss@x" },
  isManager: true,
}));

vi.mock("@/hooks/use-mobile", () => ({ useIsMobile: () => mocks.layout === "phone" }));
vi.mock("@/lib/portal/use-shell-layout", () => ({ useShellLayout: () => mocks.layout }));
vi.mock("@/lib/portal/use-active-zone", () => ({ useActiveZone: () => mocks.zone }));
vi.mock("@/lib/portal/use-viewer-is-manager", () => ({
  useViewerIsManager: () => ({ isManager: mocks.isManager, isPending: false }),
}));

vi.mock("@/components/org-tree", () => ({ OrgTree: () => <div /> }));
vi.mock("@/components/mock-banner", () => ({ MockBanner: () => <div /> }));
vi.mock("@/components/view-as-banner", () => ({
  ViewAsBanner: () => <div data-testid="view-as-banner" />,
}));
vi.mock("@/components/portal/zone-content", () => ({ ZoneContent: () => <div /> }));
vi.mock("@/components/portal/scope-select", () => ({ ScopeSelect: () => <div /> }));
vi.mock("@/components/portal/slice-select", () => ({ SliceSelect: () => <div /> }));
vi.mock("@/components/widgets/period-selector-bar", () => ({
  PeriodSelectorBar: () => <div />,
}));
vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({ data: null }),
}));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    period: "month",
    customRange: null,
    setPeriod: vi.fn(),
    setCustomRange: vi.fn(),
  }),
}));
vi.mock("@/auth", () => ({ useViewer: () => ({ email: "boss@x" }) }));
// The settings menu pulls in viewer/theme/i18n plumbing; its presence is what
// matters here — on a phone it is only reachable through this drawer.
vi.mock("@/components/app-sidebar-footer", () => ({
  AppSidebarFooter: () => <div data-testid="settings-menu" />,
}));

import { SidebarProvider, SidebarTrigger } from "@/components/ui/sidebar";
import {
  usePortalItem,
  usePortalZone,
} from "@/lib/portal/portal-nav";
import { act, renderHook } from "@testing-library/react";

import { ContextPane } from "./context-pane";
import { LensRail } from "./lens-rail";

/** The real shell wiring: trigger in the topbar, pane as the drawer. */
function Shell() {
  return (
    <SidebarProvider>
      <LensRail />
      <SidebarTrigger />
      <ContextPane />
    </SidebarProvider>
  );
}

beforeEach(() => {
  mocks.layout = "phone";
  mocks.zone = { activeZone: "overview", activePerson: "boss@x" };
  mocks.isManager = true;
  act(() => {
    portalRouter.set({ item: undefined });
  });
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
});

const openDrawer = async () => {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: /sidebar/i }));
  return user;
};

describe("shell layout: phone", () => {
  it("hides the icon rail — it would eat 56px of a 375px screen", () => {
    const { container } = render(
      <SidebarProvider>
        <LensRail />
      </SidebarProvider>,
    );
    // The provider wrapper is all that renders; the rail itself contributes no
    // sidebar element.
    expect(container.querySelector('[data-slot="sidebar"]')).toBeNull();
  });

  it("keeps the rail on a tablet — 56px is affordable there", () => {
    mocks.layout = "narrow";
    const { container } = render(
      <SidebarProvider>
        <LensRail />
      </SidebarProvider>,
    );
    expect(container.querySelector('[data-slot="sidebar"]')).not.toBeNull();
  });

  it("keeps the rail on desktop", () => {
    mocks.layout = "wide";
    const { container } = render(
      <SidebarProvider>
        <LensRail />
      </SidebarProvider>,
    );
    expect(container.querySelector('[data-slot="sidebar"]')).not.toBeNull();
  });

  it("keeps the pane closed until the topbar trigger opens it", async () => {
    render(<Shell />);
    expect(screen.queryByText("At a glance")).not.toBeInTheDocument();
    await openDrawer();
    expect(screen.getByText("At a glance")).toBeInTheDocument();
  });

  it("shows the zone switcher collapsed, so sections stay above the fold", async () => {
    render(<Shell />);
    await openDrawer();
    // One row for the active zone; the other zones are not listed yet.
    expect(screen.getByRole("button", { name: "Overview", expanded: false })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "AI & Cost" })).not.toBeInTheDocument();
    // …and the zone's own sections are already there.
    expect(screen.getByText("Trend")).toBeInTheDocument();
  });

  it("expands the zone list on demand and re-collapses after a pick", async () => {
    render(<Shell />);
    const user = await openDrawer();
    const zoneState = renderHook(() => usePortalZone());

    await user.click(screen.getByRole("button", { name: "Overview", expanded: false }));
    const aiCost = screen.getByRole("button", { name: "AI & Cost" });

    await user.click(aiCost);
    expect(zoneState.result.current).toBe("aicost");
    // Collapsed again: "Manage" only exists while the list is expanded, so its
    // absence is the collapse. The drawer itself stays open — the Settings row
    // is still there — so the new zone's sections are what the reader sees next.
    expect(screen.queryByRole("button", { name: "Manage" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
  });

  it("closes the drawer when a section is picked — otherwise it hides the view", async () => {
    render(<Shell />);
    const user = await openDrawer();
    const itemState = renderHook(() => usePortalItem());

    await user.click(screen.getByText("Trend"));

    expect(itemState.result.current).toBe("trend");
    expect(screen.queryByText("Trend")).not.toBeInTheDocument();
  });

  it("keeps the settings menu one tap away instead of inline", async () => {
    // Inline it costs six of the ~14 rows a phone has, which is what the
    // sections need. Behind one row it costs one — the same trade the desktop
    // rail makes with its settings icon.
    render(<Shell />);
    const user = await openDrawer();
    expect(screen.queryByTestId("settings-menu")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByTestId("settings-menu")).toBeInTheDocument();
  });

  it("drops the header — the zone row already names the zone", async () => {
    render(<Shell />);
    await openDrawer();
    expect(screen.queryByText("Cross-functional org rollup")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Overview", expanded: false })).toBeInTheDocument();
  });

  it("adds neither the zone switcher nor the settings menu on desktop", () => {
    mocks.layout = "wide";
    render(
      <SidebarProvider>
        <ContextPane />
      </SidebarProvider>,
    );
    // Desktop keeps its header, and the rail's duties stay in the rail.
    const pane = screen.getByText("Cross-functional org rollup").closest("[data-slot='sidebar']");
    expect(screen.getByText("At a glance")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Settings" })).not.toBeInTheDocument();
    expect(
      within(pane as HTMLElement).queryByRole("button", { name: "Overview", expanded: false }),
    ).not.toBeInTheDocument();
  });
});

describe("shell layout: narrow (tablet)", () => {
  beforeEach(() => {
    mocks.layout = "narrow";
  });

  it("collapses the pane off-canvas instead of Sheeting it", () => {
    // The rail is still there, so the pane must NOT take over the rail's duties
    // — no zone list, no settings row, and the header stays.
    render(<Shell />);
    const pane = screen
      .getByText("Cross-functional org rollup")
      .closest("[data-slot='sidebar']") as HTMLElement;
    // Settings still exist — in the RAIL, where they belong at this width.
    expect(within(pane).queryByRole("button", { name: "Settings" })).not.toBeInTheDocument();
    expect(
      within(pane).queryByRole("button", { name: "Overview", expanded: false }),
    ).not.toBeInTheDocument();
  });

  it("closes the pane after a section pick, same as the phone drawer", async () => {
    const user = userEvent.setup();
    render(<Shell />);
    const itemState = renderHook(() => usePortalItem());

    // The pane starts expanded in this bare harness (no PaneStateForLayout), so
    // the sections are reachable; picking one must collapse it.
    await user.click(screen.getByText("Trend"));

    expect(itemState.result.current).toBe("trend");
    expect(document.querySelector('[data-state="collapsed"]')).not.toBeNull();
  });
});

describe("the global controls stay reachable while reading", () => {
  it("pins the topbar to the scroll container, opaquely", async () => {
    const { PortalTopBar } = await import("./portal-topbar");
    const { container } = render(
      <SidebarProvider>
        <PortalTopBar />
      </SidebarProvider>,
    );
    const bar = container.querySelector("div.sticky");
    // Sticky alone is not enough: content scrolling under a transparent bar
    // makes both unreadable, and a bar below the cards' stacking order is
    // covered by them.
    expect(bar?.className).toContain("top-0");
    expect(bar?.className).toContain("bg-background");
    expect(bar?.className).toMatch(/z-\d+/);
  });
});

describe("what the shell must never drop", () => {
  it("renders the impersonation banner — a view-as session needs its exit", async () => {
    // It went missing when the portal became the shell: an operator viewing as
    // someone else saw no indication whose data was on screen, and no way back.
    // The banner's own behaviour is covered by its tests; this pins that the
    // shell still composes it in.
    const { PortalLayout } = await import("./portal-layout");
    render(<PortalLayout />);
    expect(screen.getByTestId("view-as-banner")).toBeInTheDocument();
  });
});
