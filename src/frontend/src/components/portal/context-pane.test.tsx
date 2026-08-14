// @vitest-environment jsdom
/**
 * ContextPane semantics: the second navigation level follows the active zone
 * (theme items for Overview, direction→lens tree for Directions, roster/org
 * items for People, catalog items for Manage), and clicking writes the
 * portal-store selection the content area renders from.
 */
vi.mock("@/queries/identity-me", () => ({
  useIsAdmin: () => ({ isAdmin: false, isPending: false }),
}));
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
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
}));

vi.mock("@/lib/portal/use-active-zone", () => ({ useActiveZone: () => mocks.zone }));
vi.mock("@/components/org-tree", () => ({
  OrgTree: () => <div data-testid="org-tree" />,
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
vi.mock("@/lib/portal/use-person-sections", () => ({
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
    await userEvent.click(screen.getByText("What we can see"));
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
    expect(screen.getByText("Roster & org structure")).toBeInTheDocument();
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

  it("highlights nothing in Manage, whose no-item view is no menu entry", () => {
    mocks.zone = { activeZone: "manage", activePerson: "boss@x" };
    pane();
    expect(buttonFor("Metric catalog")).not.toHaveAttribute("data-active");
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
        phrase: "no peer data",
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
        phrase: "no peer data",
        hasData: false,
        peersHaveData: false,
        isPending: false,
      },
    ];
    pane();
    const button = screen.getByTitle("No data reaches us for this section");
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
