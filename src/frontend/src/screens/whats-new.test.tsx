import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import "@/i18n";

import { TooltipProvider } from "@/components/ui/tooltip";
import { WhatsNewBody } from "@/screens/whats-new";

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
      <WhatsNewBody />
    </TooltipProvider>
  );
}

describe("WhatsNewBody", () => {
  it("renders the release header and stamp", () => {
    renderScreen();
    expect(
      screen.getByRole("heading", { name: "What's new · November 2026" })
    ).toBeInTheDocument();
    expect(screen.getByText("26.11")).toBeInTheDocument();
    expect(screen.queryByText("Highlights")).not.toBeInTheDocument();
    expect(screen.queryByText("Focus")).not.toBeInTheDocument();
  });

  it("groups the release into sections, as the written notes do", () => {
    renderScreen();
    const release = within(sectionFor("Improvements you'll notice"));
    for (const title of [
      "Dashboards",
      "Connectors",
      "AI & Cost",
      "Collaboration",
      "New UI",
    ]) {
      expect(release.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    for (const title of [
      "Custom dashboards",
      "Allure TestOps",
      "YouTrack",
      "GitLab",
      "Codex spend",
      "Meeting counts",
      "New sidebar",
      "Home",
    ]) {
      expect(release.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    // The section names the area, so entries no longer repeat it as a category
    // label of their own.
    expect(screen.queryByText("Git output")).not.toBeInTheDocument();
  });

  it("tells who builds dashboards, how they share them and file them", () => {
    renderScreen();
    const entry = screen
      .getByRole("heading", { name: "Custom dashboards" })
      .closest("article");
    expect(entry).toHaveTextContent(/^Custom dashboardsBuild custom metrics/);
    expect(entry).toHaveTextContent("through MCP");
    expect(entry).toHaveTextContent("Share them to view or edit");
    expect(entry).toHaveTextContent("folders and tags");
    expect(entry).toHaveTextContent("Building requires administrator access.");
  });

  it("keeps smaller changes in a closed Other changes and fixes block", async () => {
    const user = userEvent.setup();
    renderScreen();

    const release = within(sectionFor("Improvements you'll notice"));
    const toggle = release.getByRole("button", {
      name: /Other changes and fixes/,
    });
    expect(screen.queryByRole("listitem")).not.toBeInTheDocument();

    await user.click(toggle);

    expect(
      release.getAllByRole("listitem").map((bullet) => bullet.textContent)
    ).toEqual([
      "Added an expanded view for widgets.",
      "Added a time range picker to each custom dashboard.",
      "Gave Collaboration metrics clearer names: Focus Time is now Time Outside Meetings, and Email Read Actions is now Email Read Events.",
      "Split Meetings by kind and removed the Scheduled and Ad-hoc Meetings cards.",
    ]);
  });

  it("puts collaboration right after dashboards", () => {
    renderScreen();
    const groups = [
      "Dashboards",
      "Connectors",
      "AI & Cost",
      "Collaboration",
      "New UI",
    ];
    const order = within(sectionFor("Improvements you'll notice"))
      .getAllByRole("heading", { level: 4 })
      .map((heading) => heading.textContent)
      .filter((title) => groups.includes(title ?? ""));
    expect(order).toEqual([
      "Dashboards",
      "Collaboration",
      "Connectors",
      "AI & Cost",
      "New UI",
    ]);
  });

  it("puts the new UI last", () => {
    renderScreen();
    const headings = within(sectionFor("Improvements you'll notice"))
      .getAllByRole("heading", { level: 4 })
      .map((heading) => heading.textContent);
    expect(headings.at(-3)).toBe("New UI");
  });

  it("leaves out alerts, which not every installation ships", () => {
    renderScreen();
    expect(screen.queryByText(/alert/i)).not.toBeInTheDocument();
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
    const august = screen.getByRole("button", {
      name: /What's new — 31 August 2026/,
    });
    expect(august).toHaveTextContent("26.08");
    expect(august).toHaveTextContent("13 improvements");
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

  it("carries the August 31 release's own sections and entries", async () => {
    const user = userEvent.setup();
    renderScreen();

    const entry = screen.getByRole("button", {
      name: /What's new — 31 August 2026/,
    });
    expect(
      screen.queryByRole("heading", { name: "The pages the portal has now" })
    ).not.toBeInTheDocument();

    await user.click(entry);

    const archived = within(entry.parentElement as HTMLElement);
    for (const title of [
      "New UI",
      "Git",
      "Connectors",
      "Identity",
      "Data health",
      "Reports",
      "Sign-in",
    ]) {
      expect(
        archived.getByRole("heading", { name: title })
      ).toBeInTheDocument();
    }
    for (const title of [
      "The pages the portal has now",
      "Git data comes through our own proxy",
      "Jira: the whole data set, not just open issues",
      "Claude Team: seats and spend collected",
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
