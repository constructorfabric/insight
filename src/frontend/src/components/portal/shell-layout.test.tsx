// @vitest-environment jsdom
/**
 * Shell semantics per width tier. The rail and the pane are two columns of one
 * collapsible sidebar: on a phone both live in the drawer, on a tablet the
 * sidebar collapses to the rail. A zone pick keeps the pane in view while a
 * section pick gets it out of the way.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import "@/i18n";

import { portalRouter } from "@/test/portal-router";

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  layout: "phone" as "phone" | "narrow" | "wide",
  zone: { activeZone: "overview", activePerson: "boss@x" },
  isManager: true,
  openFeedback: vi.fn(),
}));

vi.mock("@/components/feedback-context", () => ({
  useFeedbackDialog: () => ({ openFeedback: mocks.openFeedback }),
}));

vi.mock("@/lib/portal/use-shell-layout", () => ({ useShellLayout: () => mocks.layout }));
vi.mock("@/lib/portal/use-active-zone", () => ({ useActiveZone: () => mocks.zone }));
vi.mock("@/queries/identity-me", () => ({
  useIsAdmin: () => ({ isAdmin: false, isPending: false }),
  useVisibilityPolicy: () => ({
    policy: "org_chart",
    isFlat: false,
    isPending: false,
  }),
}));
vi.mock("@/lib/portal/use-viewer-is-manager", () => ({
  useViewerIsManager: () => ({ isManager: mocks.isManager, isPending: false }),
}));

vi.mock("@/components/org-tree", () => ({ OrgTree: () => <div /> }));
vi.mock("@/components/mock-banner", () => ({ MockBanner: () => <div /> }));
vi.mock("@/components/view-as-banner", () => ({
  ViewAsBanner: () => <div data-testid="view-as-banner" />,
}));
vi.mock("@/components/portal/zone-content", () => ({ ZoneContent: () => <div /> }));
vi.mock("@/components/portal/custom-nav", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/portal/custom-nav")>()),
  HomeDashboardsGroup: () => null,
}));
vi.mock("@/components/portal/slice-select", () => ({ SliceSelect: () => <div /> }));
vi.mock("@/components/widgets/period-selector-bar", () => ({
  PeriodSelectorBar: () => <div />,
}));
vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({ data: null }),
}));
vi.mock("@/queries/visible-roster", () => ({
  useVisibleRoster: () => ({
    roster: [],
    isPending: false,
    isError: false,
    retry: () => {},
  }),
}));
// The topbar asks the catalog which attributes a comparison may use; this
// test is about the shell's layout, not about that answer.
vi.mock("@/queries/metric-definitions", () => ({
  useMetricDefinitionsResponse: () => ({ data: undefined }),
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
  AppSidebarFooter: ({
    onNavigate,
    showFeedback = true,
  }: {
    onNavigate?: () => void;
    showFeedback?: boolean;
  }) => (
    <div data-testid="settings-menu" data-shows-feedback={String(showFeedback)}>
      <button type="button" onClick={onNavigate}>
        Go somewhere
      </button>
    </div>
  ),
}));

import { SidebarProvider, SidebarTrigger } from "@gears-frontx/ui-kit";
import {
  usePortalItem,
  usePortalZone,
} from "@/lib/portal/portal-nav";
import { act, renderHook } from "@testing-library/react";

import { ShellSidebar } from "./portal-layout";

/** The real shell wiring: trigger in the topbar, rail and pane in one sidebar. */
function Shell() {
  return (
    <SidebarProvider>
      <ShellSidebar />
      <SidebarTrigger />
    </SidebarProvider>
  );
}

const VIEWPORT_WIDTH_PX = { phone: 375, narrow: 900, wide: 1280 } as const;

beforeEach(() => {
  mocks.layout = "phone";
  mocks.zone = { activeZone: "overview", activePerson: "boss@x" };
  mocks.isManager = true;
  mocks.openFeedback.mockClear();
  act(() => {
    portalRouter.set({ item: undefined });
  });
  // The kit's sidebar decides phone vs desktop from the viewport width.
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    get: () => VIEWPORT_WIDTH_PX[mocks.layout],
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

/** The context pane's own column, around its header and sections. */
const sectionsPane = () =>
  screen.getByRole("navigation", { name: "Section navigation", hidden: true })
    .parentElement as HTMLElement;

const openDrawer = async () => {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: /sidebar/i }));
  return user;
};

describe("shell layout: phone", () => {
  it("keeps the drawer closed until the topbar trigger opens it, rail and pane together", async () => {
    render(<Shell />);
    expect(screen.queryByText("At a glance")).not.toBeInTheDocument();

    await openDrawer();

    expect(screen.getByRole("button", { name: "Explore" })).toBeInTheDocument();
    expect(screen.getByText("At a glance")).toBeInTheDocument();
  });

  it("lists every rail item as a labelled row, with no icon rail", async () => {
    render(<Shell />);
    await openDrawer();

    expect(screen.queryByTestId("lens-rail")).not.toBeInTheDocument();
    for (const label of ["Home", "Dashboards", "Explore", "People", "Reports", "Manage"]) {
      expect(screen.getByRole("button", { name: label }), label).toHaveTextContent(label);
    }
  });

  it("opens no tooltip over the drawer when its first row takes focus", async () => {
    render(<Shell />);
    const user = await openDrawer();
    const home = screen.getByRole("button", { name: "Home" });

    home.focus();
    await user.hover(home);

    expect(screen.getAllByText("Home")).toHaveLength(1);
  });

  it("puts feedback and settings below the pages, in the drawer's footer", async () => {
    render(<Shell />);
    await openDrawer();
    const pages = screen.getByRole("navigation", { name: "Section navigation" });

    for (const label of ["Feedback", "Settings"]) {
      const row = screen.getByRole("button", { name: label });
      expect(
        pages.compareDocumentPosition(row) & Node.DOCUMENT_POSITION_FOLLOWING,
        label,
      ).toBeTruthy();
    }
  });

  it("keeps the drawer open on a zone pick, so the zone's sections are next", async () => {
    render(<Shell />);
    const user = await openDrawer();
    const zoneState = renderHook(() => usePortalZone());

    await user.click(screen.getByRole("button", { name: "Explore" }));

    expect(zoneState.result.current).toBe("directions");
    expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
  });

  it("closes the drawer when a section is picked — otherwise it hides the view", async () => {
    render(<Shell />);
    const user = await openDrawer();
    const itemState = renderHook(() => usePortalItem());

    await user.click(screen.getByText("By direction"));

    expect(itemState.result.current).toBe("by-direction");
    expect(screen.queryByText("By direction")).not.toBeInTheDocument();
  });

  it("keeps the settings menu one tap away instead of inline", async () => {
    render(<Shell />);
    const user = await openDrawer();
    expect(screen.queryByTestId("settings-menu")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByTestId("settings-menu")).toBeInTheDocument();
  });

  it("shuts the settings menu and the drawer once it has sent the reader somewhere", async () => {
    render(<Shell />);
    const user = await openDrawer();
    await user.click(screen.getByRole("button", { name: "Settings" }));

    await user.click(screen.getByRole("button", { name: "Go somewhere" }));

    expect(screen.queryByTestId("settings-menu")).not.toBeInTheDocument();
    expect(screen.queryByText("At a glance")).not.toBeInTheDocument();
  });

  it("closes the drawer with the feedback box, which opens behind it", async () => {
    render(<Shell />);
    const user = await openDrawer();

    await user.click(screen.getByRole("button", { name: "Feedback" }));

    expect(mocks.openFeedback).toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Settings" })).not.toBeInTheDocument();
  });

  it("leaves feedback out of the settings menu the rail already offers it beside", async () => {
    render(<Shell />);
    const user = await openDrawer();

    await user.click(screen.getByRole("button", { name: "Settings" }));

    expect(screen.getByTestId("settings-menu")).toHaveAttribute(
      "data-shows-feedback",
      "false",
    );
  });
});

describe("shell layout: narrow (tablet)", () => {
  beforeEach(() => {
    mocks.layout = "narrow";
  });

  it("keeps the rail's duties in the rail, out of the pane", () => {
    render(<Shell />);
    const pane = sectionsPane();
    expect(within(pane).queryByRole("button", { name: "Settings" })).not.toBeInTheDocument();
    expect(within(pane).queryByRole("button", { name: "Explore" })).not.toBeInTheDocument();
  });

  it("collapses to the rail after a section pick", async () => {
    const user = userEvent.setup();
    render(<Shell />);
    const itemState = renderHook(() => usePortalItem());

    // The pane starts expanded in this bare harness (no PaneStateForLayout).
    await user.click(screen.getByText("By direction"));

    expect(itemState.result.current).toBe("by-direction");
    expect(document.querySelector('[data-state="collapsed"]')).not.toBeNull();
    expect(screen.getByTestId("lens-rail")).toBeInTheDocument();
  });

  it("reopens the pane when a rail item is picked", async () => {
    const user = userEvent.setup();
    render(<Shell />);
    await user.click(screen.getByText("By direction"));

    await user.click(screen.getByRole("button", { name: "People" }));

    expect(document.querySelector('[data-state="expanded"]')).not.toBeNull();
  });
});

describe("shell layout: narrow (tablet), keyboard", () => {
  beforeEach(() => {
    mocks.layout = "narrow";
  });

  it("leaves an expanded pane in the Tab order", () => {
    render(<Shell />);
    expect(sectionsPane()).not.toHaveAttribute("inert");
  });

  it("takes a collapsed pane out of the Tab order and puts it back once opened", async () => {
    const user = userEvent.setup();
    render(<Shell />);

    await user.click(screen.getByText("By direction"));
    expect(sectionsPane(), "collapsed pane").toHaveAttribute("inert");

    await user.click(screen.getByRole("button", { name: /sidebar/i }));
    expect(sectionsPane(), "reopened pane").not.toHaveAttribute("inert");
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
