import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it } from "vitest";

import "@/i18n";

import { SidebarProvider } from "@/components/ui/sidebar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { WhatsNewScreen } from "@/screens/whats-new";

// SidebarProvider's useIsMobile reads window.matchMedia, which jsdom does
// not implement — provide a desktop-shaped stub.
beforeAll(() => {
  if (typeof window.matchMedia !== "function") {
    window.matchMedia = (query: string) =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener: () => {},
        removeEventListener: () => {},
        addListener: () => {},
        removeListener: () => {},
        dispatchEvent: () => false,
      }) as MediaQueryList;
  }
});

// Section names now repeat across the release and Coming next, so assertions
// scope themselves to the block whose header they mean.
function sectionFor(label: string): HTMLElement {
  const section = screen.getByText(label).closest("section");
  if (!section) throw new Error(`no section headed "${label}"`);
  return section;
}

function renderScreen() {
  return render(
    <TooltipProvider>
      <SidebarProvider>
        <WhatsNewScreen />
      </SidebarProvider>
    </TooltipProvider>
  );
}

describe("WhatsNewScreen", () => {
  it("renders the release header and stamp", () => {
    renderScreen();
    expect(
      screen.getByRole("heading", { name: "What's new · 31 July 2026" })
    ).toBeInTheDocument();
    expect(screen.getByText("0.4.69")).toBeInTheDocument();
    expect(screen.getByText("5 improvements")).toBeInTheDocument();
    expect(
      screen.getByText("the new interface, two new pages")
    ).toBeInTheDocument();
  });

  it("groups the release into sections, as the written notes do", () => {
    renderScreen();
    // "Platform" names a section in both the release and Coming next, so scope
    // the assertions to the release card.
    const release = within(sectionFor("Improvements you'll notice"));
    for (const title of ["New UI", "Dashboards", "Platform"]) {
      expect(release.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    for (const title of [
      "We've moved to the new interface for good",
      "Activity over time, by repository",
      "Metric catalog",
      "“No data” instead of a misleading zero",
      "Steadier data across your connectors",
    ]) {
      expect(release.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    // The section names the area, so entries no longer repeat it as a category
    // label of their own.
    expect(screen.queryByText("Git output")).not.toBeInTheDocument();
  });

  it("renders the connector entry under the Platform section", () => {
    renderScreen();
    expect(
      screen.getByText(/plus the data preparation behind them/)
    ).toBeInTheDocument();
  });

  it("states today's limitation inside each coming-next entry", () => {
    renderScreen();
    const coming = within(sectionFor("Coming next"));
    // Coming next is grouped and connected like the release above it.
    for (const title of ["Trust", "Platform"]) {
      expect(coming.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    for (const title of [
      "See the records behind a number",
      "Better people matching",
      "Compare like with like",
    ]) {
      expect(coming.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    // The separate "still on our list" callout is gone; the limitations it
    // listed have to survive inside the entries that address them.
    expect(screen.queryByText("Still on our list")).not.toBeInTheDocument();
    expect(
      screen.getByText(/email doesn't match still isn't attributed/)
    ).toBeInTheDocument();
    expect(
      screen.getByText(/but not the records themselves/)
    ).toBeInTheDocument();
  });

  it("keeps earlier releases on the page, collapsed", async () => {
    const user = userEvent.setup();
    renderScreen();

    expect(screen.getByText("Earlier releases")).toBeInTheDocument();
    const entry = screen.getByRole("button", {
      name: /What's new — 13 July 2026/,
    });
    expect(entry).toHaveTextContent("0.3.42");
    expect(entry).toHaveTextContent("9 improvements");
    expect(
      screen.queryByRole("heading", { name: "Zoom meeting data restored" })
    ).not.toBeInTheDocument();

    await user.click(entry);

    expect(
      screen.getByRole("heading", { name: "Zoom meeting data restored" })
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", {
        name: "Bitbucket pull requests now counted",
      })
    ).toBeInTheDocument();
  });

  it("groups an archived release into sections, like the current one", async () => {
    const user = userEvent.setup();
    renderScreen();

    await user.click(
      screen.getByRole("button", { name: /What's new — 13 July 2026/ })
    );

    for (const title of [
      "Team dashboards",
      "Git & code reviews",
      "Task delivery",
      "Collaboration",
      "AI adoption",
    ]) {
      expect(screen.getByRole("heading", { name: title })).toBeInTheDocument();
    }
  });
});
