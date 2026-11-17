// @vitest-environment jsdom
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

const mocks = vi.hoisted(() => ({
  selected: [] as string[],
  activeItem: "home",
  openFeedback: vi.fn(),
}));

vi.mock("@/components/feedback-context", () => ({
  useFeedbackDialog: () => ({ openFeedback: mocks.openFeedback }),
}));
vi.mock("@/lib/portal/use-rail-nav", () => ({
  useRailNav: () => ({
    items: [
      { id: "home", label: "Home", icon: () => null, zones: ["overview"] },
      { id: "people", label: "People", icon: () => null, zones: ["person", "people"] },
    ],
    activeItem: mocks.activeItem,
    selectItem: (item: { id: string }) => mocks.selected.push(item.id),
  }),
}));
vi.mock("@/components/app-sidebar-footer", () => ({
  AppSidebarFooter: ({ onNavigate }: { onNavigate?: () => void }) => (
    <button type="button" onClick={onNavigate}>
      Go somewhere
    </button>
  ),
}));

import { Sidebar, SidebarProvider, useSidebar } from "@gears-frontx/ui-kit";
import { setPortalRailLabels } from "@/lib/portal/portal-store";
import { LensRail } from "./lens-rail";

function OpenState() {
  return <output data-testid="open">{String(useSidebar().open)}</output>;
}

/** Collapsed to the rail, the way a tablet starts; open, the way wide starts. */
const rail = (open: boolean) =>
  render(
    <SidebarProvider defaultOpen={open}>
      <Sidebar collapsible="icon">
        <LensRail />
      </Sidebar>
      <OpenState />
    </SidebarProvider>,
  );
const collapsedRail = () => rail(false);

/** A label shown outside the button itself, i.e. in the tooltip popup. */
const tooltipFor = (label: string) =>
  screen
    .queryAllByText(label)
    .find((node) => !node.closest('[data-testid="lens-rail"]'));

beforeEach(() => {
  act(() => setPortalRailLabels(false));
  localStorage.clear();
  mocks.selected = [];
  mocks.activeItem = "home";
  mocks.openFeedback.mockClear();
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

describe("LensRail", () => {
  it("marks the item that owns the open zone", () => {
    mocks.activeItem = "people";
    collapsedRail();
    expect(screen.getByRole("button", { name: "People" })).toHaveAttribute("data-active");
    expect(screen.getByRole("button", { name: "Home" })).not.toHaveAttribute("data-active");
  });

  it("opens the pane beside it when an item is picked", async () => {
    const user = userEvent.setup();
    collapsedRail();
    expect(screen.getByTestId("open")).toHaveTextContent("false");

    await user.click(screen.getByRole("button", { name: "People" }));

    expect(mocks.selected).toEqual(["people"]);
    expect(screen.getByTestId("open")).toHaveTextContent("true");
  });

  it("opens the settings menu from the collapsed rail and shuts it after a pick", async () => {
    const user = userEvent.setup();
    collapsedRail();

    await user.click(screen.getByRole("button", { name: "Settings" }));
    await user.click(screen.getByRole("button", { name: "Go somewhere" }));

    expect(screen.queryByRole("button", { name: "Go somewhere" })).not.toBeInTheDocument();
  });

  it("offers feedback from the rail itself, not from inside the settings menu", async () => {
    const user = userEvent.setup();
    collapsedRail();

    await user.click(screen.getByRole("button", { name: "Feedback" }));

    expect(mocks.openFeedback).toHaveBeenCalled();
  });

  it.each([
    ["open", true],
    ["collapsed", false],
  ])("labels a rail item on hover while the pane is %s", async (_, open) => {
    const user = userEvent.setup();
    rail(open);
    expect(tooltipFor("People")).toBeUndefined();

    await user.hover(screen.getByRole("button", { name: "People" }));

    await waitFor(() => expect(tooltipFor("People")).toBeDefined());
  });

  it("labels Settings on hover and still opens its menu on a click", async () => {
    const user = userEvent.setup();
    rail(true);
    const settings = screen.getByRole("button", { name: "Settings" });

    await user.hover(settings);
    await waitFor(() => expect(tooltipFor("Settings")).toBeDefined());

    await user.click(settings);
    expect(screen.getByRole("button", { name: "Go somewhere" })).toBeInTheDocument();
  });
});

describe("LensRail names", () => {
  it("starts with icons only", () => {
    rail(true);
    expect(screen.getByRole("button", { name: "Expand navigation" })).toBeInTheDocument();
  });

  it("keeps the reader's choice across a remount", async () => {
    const user = userEvent.setup();
    const first = rail(true);

    await user.click(screen.getByRole("button", { name: "Expand navigation" }));
    first.unmount();
    rail(true);

    expect(screen.getByRole("button", { name: "Collapse navigation" })).toBeInTheDocument();
    expect(localStorage.getItem("insight.portal.railLabels")).toBe("true");
  });

  it("drops the item tooltips once the names show", async () => {
    const user = userEvent.setup();
    act(() => setPortalRailLabels(true));
    rail(true);

    await user.hover(screen.getByRole("button", { name: "People" }));

    expect(tooltipFor("People")).toBeUndefined();
  });

  it.each([
    ["icons only", false, "Expand navigation"],
    ["names shown", true, "Collapse navigation"],
  ])("labels the toggle by tooltip alone, %s", async (_, labels, name) => {
    const user = userEvent.setup();
    act(() => setPortalRailLabels(labels));
    rail(true);
    const toggle = screen.getByRole("button", { name });
    expect(toggle).toHaveTextContent(/^$/);

    await user.hover(toggle);

    await waitFor(() => expect(tooltipFor(name)).toBeDefined());
  });

  it("offers no toggle while the sidebar is collapsed to icons", () => {
    collapsedRail();
    expect(screen.queryByRole("button", { name: /(Expand|Collapse) navigation/ })).not.toBeInTheDocument();
  });
});
