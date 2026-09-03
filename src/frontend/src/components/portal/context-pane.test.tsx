// @vitest-environment jsdom
/**
 * ContextPane semantics: the second navigation level follows the active zone
 * (theme items for Overview, direction→lens tree for Directions, roster/org
 * items for People, catalog items for Manage), and clicking writes the
 * portal-store selection the content area renders from.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

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

const pane = () => render(<SidebarProvider><ContextPane /></SidebarProvider>);

const buttonFor = (label: string) => screen.getByText(label).closest("button");

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
  mocks.zone = { activeZone: "overview", activePerson: "boss@x" };
  mocks.standings = [];
  act(() => {
    portalRouter.set({ zone: undefined });
    portalRouter.set({ item: undefined });
    portalRouter.set({ dir: "dev" });
    portalRouter.set({ lens: "Delivery" });
  });
});

describe("ContextPane", () => {
  it("lists the Overview theme items and writes the selection on click", async () => {
    pane();
    expect(screen.getByText("Overview")).toBeInTheDocument();
    expect(screen.getByText("Cross-functional org rollup")).toBeInTheDocument();
    const item = renderHook(() => usePortalItem());
    await userEvent.click(screen.getByText("Data coverage"));
    expect(item.result.current).toBe("health");
  });

  it("shows the direction catalog with lenses and drives dir+lens state", async () => {
    mocks.zone = { activeZone: "directions", activePerson: "boss@x" };
    pane();
    expect(screen.getByText("Functional domains")).toBeInTheDocument();
    expect(screen.getByText("Development")).toBeInTheDocument();
    expect(screen.getByText("Collaboration")).toBeInTheDocument();

    const dir = renderHook(() => usePortalDir());
    const lens = renderHook(() => usePortalLens());
    // dev is the active dir → its lens list is expanded
    await userEvent.click(screen.getByText("Git output"));
    expect(dir.result.current).toBe("dev");
    expect(lens.result.current).toBe("Git output");
  });

  it("expands a direction and its first lens in one navigation", async () => {
    mocks.zone = { activeZone: "directions", activePerson: "boss@x" };
    pane();
    portalRouter.navigations.length = 0;

    await userEvent.click(screen.getByText("Knowledge / Wiki"));

    expect(portalRouter.navigations).toHaveLength(1);
    expect(portalRouter.search).toMatchObject({ dir: "wiki", lens: "Overview" });
  });

  it("picks a lens in one navigation", async () => {
    mocks.zone = { activeZone: "directions", activePerson: "boss@x" };
    pane();
    portalRouter.navigations.length = 0;

    await userEvent.click(screen.getByText("Git output"));

    expect(portalRouter.navigations).toHaveLength(1);
  });

  it("switches direction when another domain is clicked", async () => {
    mocks.zone = { activeZone: "directions", activePerson: "boss@x" };
    pane();
    const dir = renderHook(() => usePortalDir());
    await userEvent.click(screen.getByText("Knowledge / Wiki"));
    expect(dir.result.current).toBe("wiki");
  });

  it("renders the People zone with the org tree and roster items", () => {
    mocks.zone = { activeZone: "people", activePerson: "boss@x" };
    pane();
    expect(screen.getByText("People & org structure")).toBeInTheDocument();
    expect(screen.getByTestId("org-tree")).toBeInTheDocument();
  });

  it("renders Manage items", () => {
    mocks.zone = { activeZone: "manage", activePerson: "boss@x" };
    pane();
    expect(screen.getByText("Catalog, identity & governance")).toBeInTheDocument();
    expect(screen.getByText(/Metric catalog/i)).toBeInTheDocument();
  });

  it.each([
    ["overview", "At a glance"],
    ["aicost", "Overview"],
    ["people", "People (roster)"],
    ["scorecard", "Fixed scorecard"],
    ["reports", "Delivery trend"],
    ["manage", "Metric catalog"],
  ])("highlights the default item of %s when the URL names none", (zone, label) => {
    mocks.zone = { activeZone: zone, activePerson: "boss@x" };
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
    mocks.zone = { activeZone: "people", activePerson: "boss@x" };
    act(() => portalRouter.set({ item: "trend" }));
    pane();
    expect(buttonFor("People (roster)")).toHaveAttribute("data-active");
  });

  it("keeps admin-only Manage items away from a non-admin", () => {
    mocks.zone = { activeZone: "manage", activePerson: "boss@x" };
    mocks.isAdmin = false;
    pane();
    expect(screen.queryByText(/Platform usage/i)).not.toBeInTheDocument();
  });

  it("shows admin-only Manage items to an admin", () => {
    mocks.zone = { activeZone: "manage", activePerson: "boss@x" };
    mocks.isAdmin = true;
    pane();
    expect(screen.getByText(/Platform usage/i)).toBeInTheDocument();
  });

  it("renders the person's sections nav in the Person zone", () => {
    mocks.zone = { activeZone: "person", activePerson: "boss@x" };
    pane();
    expect(screen.getByText("Personal metrics")).toBeInTheDocument();
    expect(screen.getByText("At a glance")).toBeInTheDocument();
  });

  it("marks a section with where the person stands in it", () => {
    // The mark is the whole point of this nav: it says which section is worth
    // opening. Its colour and its reason have to reach the reader.
    mocks.zone = { activeZone: "person", activePerson: "boss@x" };
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
    mocks.zone = { activeZone: "person", activePerson: "boss@x" };
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
    // Same grey dot for both sent readers into a section to look for work
    // that was never being measured. The hollow mark says the section itself
    // is not wired, which is not worth opening at all.
    mocks.zone = { activeZone: "person", activePerson: "boss@x" };
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
    // A pending section drawn grey would read as "nothing here" — an answer
    // the hook has not given yet.
    mocks.zone = { activeZone: "person", activePerson: "boss@x" };
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
    // And says nothing either. Both flags read false while the queries are in
    // flight, so a tooltip that trusted them would announce the strongest
    // claim of the three on an answer the hook has not given.
    expect(button.getAttribute("title")).toBeNull();
  });
});

describe("ContextPane on an organisation with no reporting lines", () => {
  it("names the People views for an organisation with no reporting lines", () => {
    mocks.isFlat = true;
    mocks.zone = { activeZone: "people", activePerson: "boss@x" };

    pane();

    expect(screen.getByText("Overview")).toBeInTheDocument();
    expect(screen.getByText("Roster")).toBeInTheDocument();
    expect(screen.queryByText("Employees")).toBeNull();
    expect(screen.queryByText("People (roster)")).toBeNull();
    expect(screen.queryByText("Median by Role")).toBeNull();
  });

  it("does not call the roster a chart", () => {
    // "WorkChart" describes a structure a flat organisation does not have, and
    // the list needs no heading of its own inside the People zone.
    mocks.isFlat = true;
    mocks.zone = { activeZone: "people", activePerson: "boss@x" };

    pane();

    expect(screen.queryByText("WorkChart")).toBeNull();
    expect(
      screen.getByLabelText("Find someone in the org"),
    ).toBeInTheDocument();
  });
});
