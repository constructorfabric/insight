// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
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
  reachPending: false,
  showPlanned: false,
  createFolder: vi.fn(),
  renameFolder: vi.fn(),
  deleteFolder: vi.fn(),
  fetchPins: vi.fn(),
  fetchDashboardRead: vi.fn(),
  fetchOrphanedFolders: vi.fn(),
}));

vi.mock("@/lib/portal/use-active-zone", () => ({ useActiveZone: () => mocks.zone }));
vi.mock("@/components/org-tree", () => ({
  OrgTree: () => <div data-testid="org-tree" />,
}));
vi.mock("@/queries/identity-me", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
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
    isPending: mocks.reachPending,
  }),
}));
vi.mock("@/auth", () => ({ useViewer: () => ({ personId: "me@x", email: "me@x" }) }));
vi.mock("@/lib/portal/portal-store", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  usePortalShowPlanned: () => mocks.showPlanned,
}));
vi.mock("@/api/custom-client", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  fetchDashboards: async (page?: { owned?: string }) => ({
    items: [
      { id: "delivery", name: "delivery", owner_person_id: null, access: "ownerless" },
    ],
    total: page?.owned === "me" ? 3 : page?.owned === "shared" ? 2 : 14,
  }),
  fetchFolders: async () => ({
    folders: [
      { id: "f1", name: "Platform", owner_person_id: "me@x", access: "owner", dashboards: 3 },
      { id: "f2", name: "Product", owner_person_id: null, access: "ownerless", dashboards: 0 },
      { id: "f3", name: "Shared", owner_person_id: "other@x", access: "viewer", dashboards: 1 },
    ],
    unfiled: 11,
  }),
  createFolder: (name: string) => mocks.createFolder(name),
  renameFolder: (id: string, name: string) => mocks.renameFolder(id, name),
  deleteFolder: (id: string) => mocks.deleteFolder(id),
  fetchPins: () => mocks.fetchPins(),
  fetchDashboardRead: (name: string) => mocks.fetchDashboardRead(name),
  fetchOrphanedFolders: () => mocks.fetchOrphanedFolders(),
}));

import { CustomApiError } from "@/api/custom-client";

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
  mocks.reachPending = false;
  mocks.showPlanned = false;
  mocks.standings = [];
  mocks.createFolder.mockReset().mockResolvedValue({ id: "f3", name: "Hiring" });
  mocks.renameFolder.mockReset().mockResolvedValue({ id: "f1", name: "Core" });
  mocks.deleteFolder.mockReset().mockResolvedValue(undefined);
  mocks.fetchPins.mockReset().mockResolvedValue([]);
  mocks.fetchDashboardRead.mockReset().mockReturnValue(new Promise(() => {}));
  mocks.fetchOrphanedFolders.mockReset().mockResolvedValue({ items: [], total: 0 });
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

  it("links an admin to their own dashboards", () => {
    mocks.isAdmin = true;
    pane();
    expect(screen.getByRole("link", { name: /My dashboards/ })).toHaveAttribute(
      "href",
      expect.stringContaining("owned=me")
    );
  });

  it("offers a member no My dashboards, since only administrators own dashboards", () => {
    pane();
    expect(screen.queryByText("My dashboards")).toBeNull();
  });

  it("shows Starter dashboards as planned only with planned sections on", () => {
    mocks.isAdmin = true;
    const { unmount } = pane();
    expect(screen.queryByText("Starter dashboards")).toBeNull();
    unmount();

    mocks.showPlanned = true;
    pane();
    expect(buttonFor("Starter dashboards")).toHaveAttribute("aria-disabled", "true");
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
  it("opens on the report builder", () => {
    inZone("reports");
    pane();
    expect(buttonFor("Report builder")).toHaveAttribute("data-active");
  });

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
  beforeEach(() => {
    mocks.isAdmin = true;
  });

  it("offers a member neither the catalogues nor what they own", async () => {
    mocks.isAdmin = false;
    inZone("custom");
    pane();

    expect(await screen.findByText("14")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Shared with me" })).toBeInTheDocument();
    for (const name of ["My dashboards", "Metrics", "Widgets", "Datasets", "Alerts"]) {
      expect(screen.queryByRole("link", { name }), `should not link: ${name}`).toBeNull();
    }
    expect(screen.queryByText("Catalogue")).toBeNull();
  });

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
    expect(screen.getByRole("link", { name: "Alerts" })).toHaveAttribute(
      "href",
      "/portal/custom/alerts",
    );
  });

  it("narrows the list to what the reader owns, or what is shared with them", () => {
    inZone("custom");
    pane();
    expect(screen.getByRole("link", { name: "My dashboards" })).toHaveAttribute(
      "href",
      "/portal/custom?owned=me",
    );
    expect(screen.getByRole("link", { name: "Shared with me" })).toHaveAttribute(
      "href",
      "/portal/custom?owned=shared",
    );
  });

  it("keeps the picked tags when narrowing by ownership", () => {
    inZone("custom");
    act(() => {
      portalRouter.go("/portal/custom");
      portalRouter.set({ tag: ["Ops"] });
    });
    pane();

    const query = new URLSearchParams(
      screen.getByRole("link", { name: "My dashboards" }).getAttribute("href")!.split("?")[1],
    );
    expect(Object.fromEntries(query)).toEqual({ owned: "me", tag: "Ops" });
  });

  it("no longer lists each saved dashboard", async () => {
    inZone("custom");
    pane();
    await screen.findByText("14");
    expect(screen.queryByText("delivery")).toBeNull();
  });

  it("shows the planned browse filter only with planned sections on", () => {
    inZone("custom");
    const { unmount } = pane();
    expect(screen.queryByText("Starred")).toBeNull();
    unmount();

    mocks.showPlanned = true;
    pane();
    expect(buttonFor("Starred")).toHaveAttribute("aria-disabled", "true");
    expect(buttonFor("Starter")).toHaveAttribute("aria-disabled", "true");
  });
});

describe("Home pane", () => {
  it("lists the org themes for a viewer with a cohort", () => {
    pane();
    expect(screen.getByText("At a glance")).toBeInTheDocument();
    expect(screen.getByText("Trend")).toBeInTheDocument();
  });

  it("holds the org themes back until the viewer's cohort is known", () => {
    mocks.canSeeOthers = false;
    mocks.reachPending = true;
    pane();
    expect(screen.queryByText("At a glance")).toBeNull();
  });

  it.each([
    ["a viewer without a cohort", false, undefined, false],
    ["a viewer looking at their team", true, undefined, true],
    ["a viewer with a cohort who picked Me", true, "me", false],
  ] as const)(
    "lists the split of work between people only for %s",
    (_case, canSeeOthers, who, shown) => {
      mocks.canSeeOthers = canSeeOthers;
      act(() => portalRouter.set({ who }));
      pane();
      expect(screen.getByText("Trend")).toBeInTheDocument();
      expect(
        screen.queryByText("Contribution breakdown") !== null,
        `case: ${_case}`,
      ).toBe(shown);
    },
  );
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

describe("Dashboards pane folders", () => {
  beforeEach(() => {
    mocks.isAdmin = true;
  });

  const onTheList = (folder?: string) => {
    inZone("custom");
    act(() => {
      portalRouter.go("/portal/custom");
      if (folder) portalRouter.set({ folder });
    });
  };

  it("lists each folder with its count, then the unfiled", async () => {
    onTheList();
    pane();

    const platform = await screen.findByRole("link", { name: /Platform/ });
    expect(platform).toHaveAttribute("href", "/portal/custom?folder=f1");
    expect(platform.closest("li")).toHaveTextContent("3");
    const unfiled = screen.getByRole("link", { name: /Unfiled/ });
    expect(unfiled).toHaveAttribute("href", "/portal/custom?folder=unfiled");
    expect(unfiled.closest("li")).toHaveTextContent("11");
  });

  it("marks the folder the URL names, and not All dashboards", async () => {
    onTheList("f1");
    pane();

    expect(await screen.findByRole("link", { name: /Platform/ })).toHaveAttribute(
      "data-active",
    );
    expect(buttonFor("Product")).not.toHaveAttribute("data-active");
    expect(buttonFor("All dashboards")).not.toHaveAttribute("data-active");
  });

  it("keeps the picked tags when switching folder", async () => {
    inZone("custom");
    act(() => {
      portalRouter.go("/portal/custom");
      portalRouter.set({ folder: "f1", tag: ["Ops"] });
    });
    pane();

    const query = (label: RegExp) =>
      new URLSearchParams(
        screen.getByRole("link", { name: label }).getAttribute("href")!.split("?")[1],
      );
    await screen.findByRole("link", { name: /Product/ });
    expect(Object.fromEntries(query(/Product/))).toEqual({ folder: "f2", tag: "Ops" });
    expect(Object.fromEntries(query(/Unfiled/))).toEqual({ folder: "unfiled", tag: "Ops" });
    expect(Object.fromEntries(query(/All dashboards/))).toEqual({ tag: "Ops" });
  });

  it("marks nothing for a folder the URL names that is not there", async () => {
    onTheList("gone");
    pane();

    await screen.findByRole("link", { name: /Platform/ });
    for (const label of ["Platform", "Product", "Unfiled", "All dashboards"]) {
      expect(buttonFor(label)).not.toHaveAttribute("data-active");
    }
  });

  it("opens the folder dialog from New folder, creating nothing yet", async () => {
    onTheList();
    pane();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: /New folder/ }));

    expect(await screen.findByRole("dialog", { name: "New folder" })).toBeVisible();
    expect(screen.queryByRole("textbox", { name: "Folder name" })).toBeNull();
    expect(mocks.createFolder).not.toHaveBeenCalled();
  });

  it("drops a rename when focus leaves an unchanged or blank name, but keeps a typed one", async () => {
    onTheList();
    pane();
    const user = userEvent.setup();
    const startRename = async () => {
      await user.click(await screen.findByRole("button", { name: "More for Platform" }));
      await user.click(await screen.findByRole("menuitem", { name: "Rename" }));
    };

    await startRename();
    await user.tab();
    expect(screen.queryByRole("textbox", { name: "Folder name" })).toBeNull();

    await startRename();
    await user.clear(screen.getByRole("textbox", { name: "Folder name" }));
    await user.tab();
    expect(screen.queryByRole("textbox", { name: "Folder name" })).toBeNull();

    await startRename();
    const field = screen.getByRole("textbox", { name: "Folder name" });
    await user.clear(field);
    await user.type(field, "Cor");
    await user.tab();
    expect(screen.getByRole("textbox", { name: "Folder name" })).toHaveValue("Cor");
    expect(mocks.renameFolder).not.toHaveBeenCalled();
  });

  it("says under the field why a new name was refused, and keeps the field", async () => {
    mocks.renameFolder.mockRejectedValue(
      new CustomApiError(409, { detail: "a folder named `product` already exists" }),
    );
    onTheList();
    pane();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "More for Platform" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename" }));
    const field = screen.getByRole("textbox", { name: "Folder name" });
    await user.clear(field);
    await user.type(field, "product{Enter}");

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "a folder named `product` already exists",
    );
    expect(screen.getByRole("textbox", { name: "Folder name" })).toHaveValue("product");
  });

  it("renames a folder from its menu, in the same field", async () => {
    onTheList();
    pane();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "More for Platform" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename" }));
    const field = screen.getByRole("textbox", { name: "Folder name" });
    expect(field).toHaveValue("Platform");
    await user.clear(field);
    await user.type(field, "Core{Enter}");

    expect(mocks.renameFolder).toHaveBeenCalledWith("f1", "Core");
  });

  it("offers each folder only what the caller's access to it allows", async () => {
    onTheList();
    pane();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "More for Platform" }));
    expect(
      (await screen.findAllByRole("menuitem")).map((item) => item.textContent),
    ).toEqual(["Rename", "Share…", "Delete"]);
    await user.keyboard("{Escape}");

    await user.click(screen.getByRole("button", { name: "More for Product" }));
    expect(
      (await screen.findAllByRole("menuitem")).map((item) => item.textContent),
    ).toEqual(["Rename", "Delete"]);
    expect(screen.queryByRole("button", { name: "More for Shared" })).toBeNull();
  });

  it("offers a member no sharing of a folder they own", async () => {
    mocks.isAdmin = false;
    onTheList();
    pane();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "More for Platform" }));
    expect(
      (await screen.findAllByRole("menuitem")).map((item) => item.textContent),
    ).toEqual(["Rename", "Delete"]);
  });

  it("asks before deleting a folder, and says its dashboards stay", async () => {
    onTheList();
    pane();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "More for Platform" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete" }));

    expect(await screen.findByRole("dialog")).toHaveTextContent(
      "Its dashboards are not deleted.",
    );
    expect(mocks.deleteFolder).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Delete folder" }));
    expect(mocks.deleteFolder).toHaveBeenCalledWith("f1");
  });

  it("marks an orphaned folder and offers to transfer it", async () => {
    mocks.fetchOrphanedFolders.mockResolvedValue({
      items: [
        {
          id: "f9",
          name: "Handover",
          owner_person_id: "left@x",
          updated_at: "2026-08-15T00:00:00.000Z",
        },
      ],
      total: 1,
    });
    onTheList();
    pane();
    const user = userEvent.setup();

    const row = await screen.findByRole("button", { name: /Handover/ });
    expect(row).toHaveTextContent("Orphaned");
    expect(screen.queryByRole("link", { name: /Handover/ })).toBeNull();
    await user.click(row);

    expect(await screen.findByRole("dialog")).toHaveTextContent("Transfer Handover");
  });

  it("asks nothing about orphaned folders for a member", async () => {
    mocks.isAdmin = false;
    onTheList();
    pane();

    expect(await screen.findByRole("link", { name: /Platform/ })).toBeInTheDocument();
    expect(mocks.fetchOrphanedFolders).not.toHaveBeenCalled();
  });
});

describe("Dashboards pane pins", () => {
  beforeEach(() => {
    mocks.isAdmin = true;
  });

  const TITLES: Record<string, string> = {
    hiring: "Hiring pipeline",
    delivery: "Delivery",
  };

  function pinned(ids: string[]) {
    mocks.fetchPins.mockResolvedValue(ids.map((id) => ({ id, name: id })));
    mocks.fetchDashboardRead.mockImplementation(async (id: string) => ({
      name: id,
      body: { title: TITLES[id] ?? "", widgets: [] },
      tags: [],
      ownerPersonId: null,
      ownerName: null,
      access: "ownerless",
    }));
    inZone("custom");
  }

  const groupLabels = () =>
    [...document.querySelectorAll('[data-slot="sidebar-group-label"]')].map(
      (label) => label.textContent,
    );

  const pinnedRows = () =>
    within(
      screen
        .getByText("Pinned")
        .closest('[data-slot="sidebar-group"]') as HTMLElement,
    ).getAllByRole("link");

  it("lists the pinned dashboards after the catalogues, in pin order, by title", async () => {
    pinned(["hiring", "delivery"]);
    pane();

    await screen.findByRole("link", { name: "Hiring pipeline" });
    expect(groupLabels()).toEqual(["Browse", "Folders", "Catalogue", "Pinned"]);
    const rows = pinnedRows();
    expect(rows.map((row) => row.textContent)).toEqual([
      "Hiring pipeline",
      "Delivery",
    ]);
    expect(rows.map((row) => row.getAttribute("href"))).toEqual([
      "/portal/custom/hiring",
      "/portal/custom/delivery",
    ]);
  });

  it("names a pinned dashboard by its name while its title loads or when it has none", async () => {
    mocks.fetchPins.mockResolvedValue([
      { id: "d-loading", name: "Loading board" },
      { id: "d-untitled", name: "Untitled board" },
    ]);
    mocks.fetchDashboardRead.mockImplementation((id: string) =>
      id === "d-loading"
        ? new Promise(() => {})
        : Promise.resolve({
            name: "Untitled board",
            body: { title: "", widgets: [] },
            tags: [],
            ownerPersonId: null,
            ownerName: null,
            access: "ownerless",
          }),
    );
    inZone("custom");
    pane();

    await screen.findByRole("link", { name: "Loading board" });
    await waitFor(() =>
      expect(mocks.fetchDashboardRead).toHaveBeenCalledWith("d-untitled"),
    );
    expect(pinnedRows().map((row) => row.textContent)).toEqual([
      "Loading board",
      "Untitled board",
    ]);
    expect(pinnedRows().map((row) => row.getAttribute("href"))).toEqual([
      "/portal/custom/d-loading",
      "/portal/custom/d-untitled",
    ]);
  });

  it("marks the pinned dashboard on screen", async () => {
    pinned(["hiring", "delivery"]);
    act(() => portalRouter.go("/portal/custom/delivery"));
    pane();

    expect(
      await screen.findByRole("link", { name: "Delivery" }),
    ).toHaveAttribute("data-active");
    expect(
      screen.getByRole("link", { name: "Hiring pipeline" }),
    ).not.toHaveAttribute("data-active");
  });

  it("hides the group when nothing is pinned", async () => {
    pinned([]);
    pane();

    await screen.findByText("14");
    await waitFor(() => expect(mocks.fetchPins).toHaveBeenCalled());
    expect(screen.queryByText("Pinned")).toBeNull();
  });
});
