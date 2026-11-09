// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
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
import { LensRail } from "./lens-rail";

function OpenState() {
  return <output data-testid="open">{String(useSidebar().open)}</output>;
}

/** Collapsed to the rail, the way a tablet starts. */
const collapsedRail = () =>
  render(
    <SidebarProvider defaultOpen={false}>
      <Sidebar collapsible="icon">
        <LensRail />
      </Sidebar>
      <OpenState />
    </SidebarProvider>,
  );

beforeEach(() => {
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

    await user.click(screen.getByRole("button", { name: "Send feedback" }));

    expect(mocks.openFeedback).toHaveBeenCalled();
  });
});
