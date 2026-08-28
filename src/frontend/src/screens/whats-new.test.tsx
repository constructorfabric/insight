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
      screen.getByRole("heading", { name: "What's new · 31 August 2026" })
    ).toBeInTheDocument();
    expect(screen.getByText("26.08")).toBeInTheDocument();
    expect(screen.getByText("13 improvements")).toBeInTheDocument();
    expect(
      screen.getByText("more data coming in, and the new UI over it")
    ).toBeInTheDocument();
  });

  it("groups the release into sections, as the written notes do", () => {
    renderScreen();
    // "Platform" names a section in both the release and Coming next, so scope
    // the assertions to the release card.
    const release = within(sectionFor("Improvements you'll notice"));
    for (const title of [
      "New UI",
      "Sign-in",
      "Git",
      "Connectors",
      "Identity",
      "Data health",
      "Reports",
    ]) {
      expect(release.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    for (const title of [
      "The pages the portal has now",
      "Git data comes through our own proxy",
      "Git output split by what reached the default branch",
      "Jira: the whole data set, not just open issues",
      "Data health — when each source last synced",
      "See the records behind a number",
      "Claude Team: seats and spend collected",
    ]) {
      expect(release.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    // The section names the area, so entries no longer repeat it as a category
    // label of their own.
    expect(screen.queryByText("Git output")).not.toBeInTheDocument();
  });

  it("renders the data-health entry", () => {
    renderScreen();
    expect(
      screen.getByText(/lists each source with the state of its last sync/)
    ).toBeInTheDocument();
  });

  it("carries no forward-looking section", () => {
    renderScreen();
    expect(screen.queryByText("Coming next")).not.toBeInTheDocument();
    expect(screen.queryByText("Still on our list")).not.toBeInTheDocument();
  });

  it("keeps earlier releases on the page, collapsed", async () => {
    const user = userEvent.setup();
    renderScreen();

    expect(screen.getByText("Earlier releases")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /What's new — 31 July 2026/ })
    ).toHaveTextContent("0.4.69");
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

    const entry = screen.getByRole("button", {
      name: /What's new — 13 July 2026/,
    });
    await user.click(entry);

    const archived = within(entry.parentElement as HTMLElement);
    for (const title of [
      "Team dashboards",
      "Git & code reviews",
      "Task delivery",
      "Collaboration",
      "AI adoption",
    ]) {
      expect(
        archived.getByRole("heading", { name: title })
      ).toBeInTheDocument();
    }
  });

  it("carries the July 31 release's own sections and entries", async () => {
    const user = userEvent.setup();
    renderScreen();

    const entry = screen.getByRole("button", {
      name: /What's new — 31 July 2026/,
    });
    await user.click(entry);

    const archived = within(entry.parentElement as HTMLElement);
    for (const title of ["New UI", "Dashboards", "Platform"]) {
      expect(
        archived.getByRole("heading", { name: title })
      ).toBeInTheDocument();
    }
    for (const title of [
      "We've moved to the new interface for good",
      "Activity over time, by repository",
      "Metric catalog",
      "“No data” instead of a misleading zero",
      "Steadier data across your connectors",
    ]) {
      expect(
        archived.getByRole("heading", { name: title })
      ).toBeInTheDocument();
    }
  });
});
