// @vitest-environment jsdom
/**
 * The rail's open state.
 *
 * Every case here is about the interaction rather than the look, because the
 * look is the easy half. The one that matters is the click: a click navigates
 * and leaves the pointer sitting on the rail, so without an explicit dismissal
 * the rail reopens on top of the pane the click was aimed at. That failed
 * silently once already when the state was expressed as CSS variants — the
 * rules simply never matched and nothing said so.
 */
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  layout: "wide" as "phone" | "narrow" | "wide",
  selected: [] as string[],
}));

vi.mock("@/lib/portal/use-shell-layout", () => ({
  useShellLayout: () => mocks.layout,
}));
vi.mock("@/lib/portal/use-zone-nav", () => ({
  useZoneNav: () => ({
    zones: [
      { id: "overview", label: "Overview", icon: () => null },
      { id: "people", label: "People", icon: () => null },
    ],
    activeZone: "overview",
    selectZone: (z: { id: string }) => mocks.selected.push(z.id),
  }),
}));
vi.mock("@/components/app-sidebar-footer", () => ({
  AppSidebarFooter: ({ onNavigate }: { onNavigate?: () => void }) => (
    <button type="button" onClick={onNavigate}>
      Go somewhere
    </button>
  ),
}));

import { SidebarProvider } from "@/components/ui/sidebar";
import { LensRail } from "./lens-rail";

/** The rail opens on a timer, so a hover only counts once the wait is over. */
const settle = () => act(() => { vi.advanceTimersByTime(400); });

const rail = () =>
  render(
    <SidebarProvider>
      <LensRail />
    </SidebarProvider>,
  );

/** The label is present either way; what changes is whether it can be seen. */
const labelOf = (name: string) =>
  screen.getByRole("button", { name }).querySelector("span:not(.sr-only)");

beforeEach(() => {
  // `shouldAdvanceTime` is required, not incidental: `userEvent` awaits a real
  // promise between events, and with a clock that only moves when a test moves
  // it that await is never resolved — every pointer interaction here hangs
  // until the runner's timeout. It does mean the opening wait can expire on
  // its own mid-test, which is harmless: the next action either wanted it open
  // anyway, or is a leave, and a leave cancels the pending timer as it closes.
  vi.useFakeTimers({ shouldAdvanceTime: true });
  mocks.layout = "wide";
  mocks.selected = [];
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
  it("shows labels while the pointer is on it", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    rail();
    expect(labelOf("Overview")).toHaveClass("opacity-0");

    await user.hover(screen.getByTestId("lens-rail"));
    settle();
    expect(labelOf("Overview")).toHaveClass("opacity-100");
  });

  it("collapses on a click and stays collapsed under the pointer", async () => {
    // The whole reason this state exists. The click navigates; the pointer has
    // not moved; reopening here would cover the pane that was just asked for.
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    rail();
    await user.hover(screen.getByTestId("lens-rail"));
    settle();
    expect(labelOf("People")).toHaveClass("opacity-100");

    await user.click(screen.getByRole("button", { name: "People" }));
    expect(mocks.selected).toEqual(["people"]);
    expect(labelOf("People")).toHaveClass("opacity-0");
  });

  it("collapses when the settings menu navigates, for the same reason", async () => {
    // The trigger lives in the rail's own footer, so reaching it expands the
    // rail over the pane the destination renders in.
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    rail();
    await user.hover(screen.getByTestId("lens-rail"));
    settle();
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(labelOf("People")).toHaveClass("opacity-100");

    await user.click(screen.getByRole("button", { name: "Go somewhere" }));

    expect(labelOf("People")).toHaveClass("opacity-0");
  });

  it("expands again once the pointer has left and come back", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    rail();
    const el = screen.getByTestId("lens-rail");

    await user.hover(el);
    settle();
    await user.click(screen.getByRole("button", { name: "People" }));
    expect(labelOf("People")).toHaveClass("opacity-0");

    await user.unhover(el);
    await user.hover(el);
    settle();
    expect(labelOf("People")).toHaveClass("opacity-100");
  });

  it("renders nothing on a phone", () => {
    // 56px of rail plus a 256px pane left a phone with almost no content; the
    // zones live in the context pane's drawer there instead.
    mocks.layout = "phone";
    rail();
    expect(screen.queryByTestId("lens-rail")).not.toBeInTheDocument();
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("LensRail state that only breaks in a particular order", () => {
  it("does not strand itself when a zone is chosen from the keyboard", async () => {
    // Enter on a focused button produces a click, and a click used to mean
    // "the pointer is resting on me, stay shut until it leaves". There is no
    // pointer in this story, so nothing would ever clear that — the rail was
    // dead to the mouse from then on.
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    rail();
    await user.tab();
    await user.tab();
    await user.keyboard("{Enter}");
    expect(mocks.selected).toEqual(["people"]);

    await user.hover(screen.getByTestId("lens-rail"));
    settle();
    expect(labelOf("People")).toHaveClass("opacity-100");
  });

  it("shows the labels to a keyboard user at all", async () => {
    // Eight identical icons and the text at zero opacity is not navigable by
    // anyone who can see but is not using a pointer.
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    rail();
    await user.tab();
    await user.tab();
    expect(labelOf("Overview")).toHaveClass("opacity-100");
  });

  it("comes back shut after the rail is unmounted under the pointer", async () => {
    // A width change unmounts the rail without a pointer-leave, so the state
    // it left behind used to survive: widen again and it was already open,
    // with the pointer nowhere near it.
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const { rerender } = rail();
    await user.hover(screen.getByTestId("lens-rail"));
    settle();
    expect(labelOf("Overview")).toHaveClass("opacity-100");

    mocks.layout = "phone";
    rerender(<SidebarProvider><LensRail /></SidebarProvider>);
    mocks.layout = "wide";
    rerender(<SidebarProvider><LensRail /></SidebarProvider>);
    expect(labelOf("Overview")).toHaveClass("opacity-0");
  });

  it("keeps the labels while the keyboard is still inside it", async () => {
    // Two ways in, and only one of them is the pointer's to end. A pointer
    // crossing the rail on its way somewhere else used to collapse it under a
    // focused button, putting a keyboard user back on an unlabelled icon they
    // had no way to re-reveal.
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    rail();
    const el = screen.getByTestId("lens-rail");
    await user.tab();
    await user.tab();
    expect(labelOf("People")).toHaveClass("opacity-100");

    await user.hover(el);
    await user.unhover(el);
    settle();
    expect(labelOf("People")).toHaveClass("opacity-100");

    // Leaving with the keyboard is what ends a keyboard visit.
    act(() => (document.activeElement as HTMLElement).blur());
    expect(labelOf("People")).toHaveClass("opacity-0");
  });

  it("stays shut for a pointer that is only passing through", async () => {
    // The wait is the whole guard. Before it was a timer, the panel became
    // clickable at once and merely being over it counted as staying, so a
    // crossing pointer opened the rail anyway — over the row it was heading
    // for, having swallowed any click on the way.
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    rail();
    const el = screen.getByTestId("lens-rail");
    await user.hover(el);
    await user.unhover(el);
    settle();
    expect(labelOf("Overview")).toHaveClass("opacity-0");
  });
});
