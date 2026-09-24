// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  isFlat: false,
  zone: { activeZone: "overview", activePerson: "boss@x" },
  standings: [] as Array<{
    id: string;
    title: string;
    status: string;
    phrase: string;
    hasData: boolean;
    peersHaveData: boolean;
    isPending: boolean;
  }>,
  isAdmin: false,
  canSeeOthers: true,
  showPlanned: false,
}));

vi.mock("@/lib/portal/use-active-zone", () => ({ useActiveZone: () => mocks.zone }));
vi.mock("@/components/org-tree", () => ({
  OrgTree: () => <div data-testid="org-tree" />,
}));
vi.mock("@/queries/identity-me", () => ({
  useIsAdmin: () => ({ isAdmin: mocks.isAdmin, isPending: false }),
  useVisibilityPolicy: () => ({
    policy: mocks.isFlat ? "flat" : "org_chart",
    isFlat: mocks.isFlat,
    isPending: false,
  }),
}));
vi.mock("@/lib/portal/use-viewer-reach", () => ({
  useViewerReach: () => ({
    canSeeOthers: mocks.canSeeOthers,
    isManager: mocks.canSeeOthers,
    isPending: false,
  }),
}));
vi.mock("@/auth", () => ({ useViewer: () => ({ personId: "me@x", email: "me@x" }) }));
vi.mock("@/lib/portal/portal-store", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  usePortalShowPlanned: () => mocks.showPlanned,
}));
vi.mock("@/api/custom-client", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  fetchDashboardNames: async () => ({ names: ["delivery"], total: 14 }),
}));

import {
  usePortalDir,
  usePortalItem,
  usePortalLens,
} from "@/lib/portal/portal-nav";
import { SidebarProvider } from "@/components/ui/sidebar";
// The sections nav asks where the person stands so it can mark each section;
// the standings come from the section screens' own queries, which this test
// has no reason to run.
// `useSelectedPersonSection` is left real: it reads the router mock above, and
// it is what decides whether "At a glance" or a section row is the active one.
vi.mock("@/lib/portal/use-person-sections", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  usePersonSectionStandings: () => mocks.standings,
}));

import { ContextPane } from "./context-pane";

const pane = () =>
  render(
    <QueryClientProvider client={new QueryClient()}>
      <SidebarProvider>
        <ContextPane />
      </SidebarProvider>
    </QueryClientProvider>,
  );

const buttonFor = (label: string) => screen.getByText(label).closest("button, a");

const inZone = (activeZone: string) => {
  mocks.zone = { activeZone, activePerson: "boss@x" };
};

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
  inZone("overview");
  mocks.isFlat = false;
  mocks.isAdmin = false;
  mocks.canSeeOthers = true;
  mocks.showPlanned = false;
  mocks.standings = [];
  act(() => {
    portalRouter.reset();
    portalRouter.set({ dir: "dev" });
    portalRouter.set({ lens: "Delivery" });
  });
});

describe("ContextPane header", () => {
  it("titles the Home pane with the product name", () => {
    pane();
    expect(screen.getByText("Constructor Insight")).toBeInTheDocument();
  });

  it.each([
    ["directions", "Explore"],
    ["aicost", "Explore"],
    ["person", "People"],
    ["people", "People"],
    ["custom", "Dashboards"],
    ["manage", "Manage"],
  ])("titles the %s pane %s", (zone, title) => {
    inZone(zone);
    pane();
    expect(screen.getByText(title, { selector: '[data-slot="sidebar-header"] *' })).toBeInTheDocument();
  });
});

describe("Home pane", () => {
  it("lists the themes and writes the selection on click", async () => {
    pane();
    const item = renderHook(() => usePortalItem());
    await userEvent.click(screen.getByText("Data coverage"));
    expect(item.result.current).toBe("health");
  });

  it("keeps the Dashboards group from a viewer who cannot open dashboards", () => {
    mocks.showPlanned = true;
    pane();
    expect(screen.queryByText("My dashboards")).toBeNull();
  });

  it("shows an admin the planned Dashboards group only with planned sections on", () => {
    mocks.isAdmin = true;
    const { unmount } = pane();
    expect(screen.queryByText("My dashboards")).toBeNull();
    unmount();

    mocks.showPlanned = true;
    pane();
    expect(buttonFor("My dashboards")).toHaveAttribute("aria-disabled", "true");
    expect(screen.queryByText("Starter dashboards")).toBeNull();
  });

  it("goes nowhere from a planned row", async () => {
    mocks.isAdmin = true;
    mocks.showPlanned = true;
    pane();

    await userEvent.click(screen.getByText("My dashboards"));

    expect(portalRouter.navigations).toHaveLength(0);
  });
});

describe("Explore pane", () => {
  it("holds the directions and the AI & Cost views together", () => {
    inZone("directions");
    pane();
    expect(screen.getByText("Development")).toBeInTheDocument();
    expect(screen.getByText("AI adoption")).toBeInTheDocument();
    expect(screen.getByText("Cost")).toBeInTheDocument();
  });

  it("shows the open direction's lenses and drives dir+lens state", async () => {
    inZone("directions");
    pane();
    const dir = renderHook(() => usePortalDir());
    const lens = renderHook(() => usePortalLens());
    await userEvent.click(screen.getByText("Git output"));
    expect(dir.result.current).toBe("dev");
    expect(lens.result.current).toBe("Git output");
  });

  it("expands a direction and its first lens in one navigation", async () => {
    inZone("directions");
    pane();
    portalRouter.navigations.length = 0;

    await userEvent.click(screen.getByText("Knowledge / Wiki"));

    expect(portalRouter.navigations).toHaveLength(1);
    expect(portalRouter.search).toMatchObject({ dir: "wiki", lens: "Overview" });
  });

  it("picks a lens in one navigation", async () => {
    inZone("directions");
    pane();
    portalRouter.navigations.length = 0;

    await userEvent.click(screen.getByText("Git output"));

    expect(portalRouter.navigations).toHaveLength(1);
  });

  it("does not open a direction while AI & Cost is on screen", () => {
    inZone("aicost");
    pane();
    expect(screen.queryByText("Git output")).toBeNull();
    expect(buttonFor("Development")).not.toHaveAttribute("data-active");
  });

  it("opens an AI & Cost view from Directions in one navigation", async () => {
    inZone("directions");
    pane();
    portalRouter.navigations.length = 0;

    await userEvent.click(screen.getByText("AI adoption"));

    expect(portalRouter.navigations).toHaveLength(1);
    expect(portalRouter.search).toMatchObject({ zone: "aicost", item: "adoption-funnel" });
  });

  it("folds the open AI & Cost group on a second click without navigating", async () => {
    inZone("aicost");
    act(() => portalRouter.set({ item: "idle-seats" }));
    pane();
    portalRouter.navigations.length = 0;

    await userEvent.click(screen.getByText("Cost"));

    expect(screen.queryByText("Idle seats")).toBeNull();
    expect(portalRouter.navigations).toHaveLength(0);
  });

  it("opens a view picked inside an AI & Cost group", async () => {
    inZone("aicost");
    act(() => portalRouter.set({ item: "idle-seats" }));
    pane();

    await userEvent.click(screen.getByText("Credits burn-down"));

    expect(portalRouter.search).toMatchObject({ zone: "aicost", item: "credits" });
  });

  it("lists the views of the open AI & Cost group", () => {
    inZone("aicost");
    act(() => portalRouter.set({ item: "idle-seats" }));
    pane();
    expect(buttonFor("Idle seats")).toHaveAttribute("data-active");
    expect(screen.queryByText("Adoption funnel")).toBeNull();
  });
});

describe("People pane", () => {
  it("offers Me and the team views", () => {
    inZone("people");
    pane();
    expect(screen.getByText("Me")).toBeInTheDocument();
    expect(screen.getByText("My team")).toBeInTheDocument();
    expect(screen.getByText("Roster · by role")).toBeInTheDocument();
    expect(screen.getByTestId("org-tree")).toBeInTheDocument();
  });

  it("opens Me on the viewer's own page", async () => {
    inZone("people");
    pane();

    await userEvent.click(screen.getByText("Me"));

    expect(portalRouter.pathname).toBe("/ic/me%40x/personal");
  });

  it("opens a team view from a person page on the viewer's team", async () => {
    inZone("person");
    pane();

    await userEvent.click(screen.getByText("Roster · by role"));

    expect(portalRouter.navigations).toHaveLength(1);
    expect(portalRouter.pathname).toBe("/ic/me%40x/team");
    expect(portalRouter.search).toMatchObject({ item: "employees" });
  });

  it("switches team view in place on a team page", async () => {
    inZone("people");
    pane();

    await userEvent.click(screen.getByText("Roster · by role"));

    expect(portalRouter.search).toMatchObject({ item: "employees" });
    expect(portalRouter.pathname).toBe("/portal");
  });

  it("offers a viewer with nobody to look at their own page only", () => {
    mocks.canSeeOthers = false;
    inZone("person");
    pane();
    expect(screen.getByText("Me")).toBeInTheDocument();
    expect(screen.queryByText("My team")).toBeNull();
  });

  it("lists the person's sections under the views", () => {
    inZone("person");
    pane();
    expect(screen.getByText("Me")).toBeInTheDocument();
    expect(screen.getByText("At a glance")).toBeInTheDocument();
    expect(screen.queryByTestId("org-tree")).toBeNull();
  });

  it.each([
    ["org chart", false],
    ["flat roster", true],
  ])("keeps the %s list in its own scroll region", (_policy, isFlat) => {
    mocks.isFlat = isFlat;
    inZone("people");

    pane();

    const search = screen.getByLabelText("Find someone in the org");
    const scrollArea = screen
      .getByTestId("org-tree")
      .closest('[data-slot="scroll-area"]');

    expect(scrollArea).not.toBeNull();
    expect(scrollArea).toHaveClass("min-h-0", "flex-1");
    expect(scrollArea).not.toContainElement(search);
    expect(scrollArea?.closest('[data-slot="sidebar-group"]')).toHaveClass(
      "min-h-0",
      "flex-1"
    );
  });

  it("marks a section with where the person stands in it", () => {
    inZone("person");
    mocks.standings = [
      {
        id: "git_output",
        title: "Git output",
        status: "bad",
        phrase: "4 of 6 behind peers",
        hasData: true,
        peersHaveData: true,
        isPending: false,
      },
    ];
    pane();
    const button = screen.getByTitle("4 of 6 behind peers");
    expect(button.querySelector(".bg-destructive")).not.toBeNull();
  });

  it("says a section has nothing rather than colouring it", () => {
    inZone("person");
    mocks.standings = [
      {
        id: "git_output",
        title: "Git output",
        status: "neutral",
        phrase: "no comparison",
        hasData: false,
        peersHaveData: true,
        isPending: false,
      },
    ];
    pane();
    const button = screen.getByTitle("No data this period");
    expect(button.querySelector(".bg-muted-foreground\\/30")).not.toBeNull();
  });

  it("marks a section nothing feeds apart from one this person is absent from", () => {
    inZone("person");
    mocks.standings = [
      {
        id: "git_output",
        title: "Git output",
        status: "neutral",
        phrase: "no comparison",
        hasData: false,
        peersHaveData: false,
        isPending: false,
      },
    ];
    pane();
    const button = screen.getByTitle("No data source is connected for this section");
    const mark = button.querySelector("span[aria-hidden]")!;
    expect(mark.className).not.toContain("bg-muted-foreground");
    expect(mark.className).toContain("border");
  });

  it("draws no mark while the standings are still loading", () => {
    inZone("person");
    mocks.standings = [
      {
        id: "git_output",
        title: "Git output",
        status: "neutral",
        phrase: "",
        hasData: false,
        peersHaveData: true,
        isPending: true,
      },
    ];
    pane();
    const button = screen.getByText("Git output").closest("button")!;
    expect(button.querySelector("span[aria-hidden]")).toBeNull();
    expect(button.getAttribute("title")).toBeNull();
  });
});

describe("People pane on an organisation with no reporting lines", () => {
  it("names the team views for an organisation with no reporting lines", () => {
    mocks.isFlat = true;
    inZone("people");

    pane();

    expect(screen.getByText("Overview")).toBeInTheDocument();
    expect(screen.getByText("Roster")).toBeInTheDocument();
    expect(screen.queryByText("My team")).toBeNull();
  });

  it("does not call the roster a chart", () => {
    mocks.isFlat = true;
    inZone("people");

    pane();

    expect(screen.queryByText("WorkChart")).toBeNull();
    expect(screen.getByLabelText("Find someone in the org")).toBeInTheDocument();
  });
});

describe("Manage pane", () => {
  it("groups the surfaces into Data, People & access and Platform", () => {
    inZone("manage");
    mocks.isAdmin = true;
    pane();
    expect(screen.getByText("Data")).toBeInTheDocument();
    expect(screen.getByText("People & access")).toBeInTheDocument();
    expect(screen.getByText("Platform")).toBeInTheDocument();
    expect(screen.getByText("Sources & connectors")).toBeInTheDocument();
  });

  it("keeps admin-only surfaces away from a non-admin", () => {
    inZone("manage");
    pane();
    expect(screen.queryByText(/Platform usage/i)).not.toBeInTheDocument();
    expect(screen.queryByText("Sources & connectors")).not.toBeInTheDocument();
  });

  it("shows admin-only surfaces to an admin", () => {
    inZone("manage");
    mocks.isAdmin = true;
    pane();
    expect(screen.getByText(/Platform usage/i)).toBeInTheDocument();
  });

  it("shows Access as planned only with planned sections on", () => {
    inZone("manage");
    const { unmount } = pane();
    expect(screen.queryByText("Access")).toBeNull();
    unmount();

    mocks.showPlanned = true;
    pane();
    expect(buttonFor("Access")).toHaveAttribute("aria-disabled", "true");
  });
});

describe("Reports pane", () => {
  it("lists what is planned and opens none of it", () => {
    inZone("reports");
    mocks.showPlanned = true;
    pane();
    for (const label of ["Snapshots", "Export PDF / HTML", "Templates"]) {
      expect(buttonFor(label)).toHaveAttribute("aria-disabled", "true");
    }
  });
});

describe("Dashboards pane", () => {
  it("counts every dashboard beside All dashboards", async () => {
    inZone("custom");
    pane();
    const all = screen.getByRole("link", { name: /All dashboards/ });
    expect(all).toHaveAttribute("href", "/portal/custom");
    expect(await screen.findByText("14")).toBeInTheDocument();
  });

  it("links the catalogues", () => {
    inZone("custom");
    pane();
    expect(screen.getByRole("link", { name: "Metrics" })).toHaveAttribute(
      "href",
      "/portal/custom/metrics",
    );
    expect(screen.getByRole("link", { name: "Widgets" })).toHaveAttribute(
      "href",
      "/portal/custom/widgets",
    );
    expect(screen.getByRole("link", { name: "Datasets" })).toHaveAttribute(
      "href",
      "/portal/custom/datasets",
    );
  });

  it("no longer lists each saved dashboard", async () => {
    inZone("custom");
    pane();
    await screen.findByText("14");
    expect(screen.queryByText("delivery")).toBeNull();
  });

  it("shows the planned browse filters only with planned sections on", () => {
    inZone("custom");
    const { unmount } = pane();
    expect(screen.queryByText("Shared with me")).toBeNull();
    unmount();

    mocks.showPlanned = true;
    pane();
    for (const label of ["My dashboards", "Shared with me", "Starred"]) {
      expect(buttonFor(label)).toHaveAttribute("aria-disabled", "true");
    }
    expect(screen.queryByText("Starter")).toBeNull();
  });
});

describe("ContextPane highlight", () => {
  it.each([
    ["overview", "At a glance"],
    ["aicost", "Overview"],
    ["people", "My team"],
    ["manage", "Data exclusions"],
  ])("highlights the default item of %s when the URL names none", (zone, label) => {
    inZone(zone);
    pane();
    expect(buttonFor(label)).toHaveAttribute("data-active");
  });

  it("moves the highlight to the item the URL names", () => {
    act(() => portalRouter.set({ item: "trend" }));
    pane();
    expect(buttonFor("Trend")).toHaveAttribute("data-active");
    expect(buttonFor("At a glance")).not.toHaveAttribute("data-active");
  });

  it("ignores an item left behind by another zone", () => {
    inZone("people");
    act(() => portalRouter.set({ item: "trend" }));
    pane();
    expect(buttonFor("My team")).toHaveAttribute("data-active");
  });
});
