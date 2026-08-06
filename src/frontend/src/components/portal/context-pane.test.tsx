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
  zone: { activeZone: "overview", activePerson: "boss@x" },
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
import { ContextPane } from "./context-pane";

const pane = () => render(<SidebarProvider><ContextPane /></SidebarProvider>);

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
    await userEvent.click(screen.getByText("Health radar"));
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

  it("renders the person's sections nav in the Person zone", () => {
    mocks.zone = { activeZone: "person", activePerson: "boss@x" };
    pane();
    expect(screen.getByText("Pick a person")).toBeInTheDocument();
    expect(screen.getByText("At a glance")).toBeInTheDocument();
  });
});
