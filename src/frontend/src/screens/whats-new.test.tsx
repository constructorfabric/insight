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
    expect(screen.getByText("22 improvements")).toBeInTheDocument();
    expect(
      screen.getByText("dashboards you build and share, and alerts on them")
    ).toBeInTheDocument();
  });

  it("groups the release into sections, as the written notes do", () => {
    renderScreen();
    const release = within(sectionFor("Improvements you'll notice"));
    for (const title of [
      "New UI",
      "Dashboards",
      "Alerts",
      "Connectors",
      "AI & Cost",
      "Collaboration",
      "Manage",
      "Fixes",
    ]) {
      expect(release.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    for (const title of [
      "Six places in the sidebar",
      "Build your own dashboards",
      "Share a dashboard, or a folder of them",
      "Get told when a metric crosses a line",
      "Allure TestOps: test results come in",
      "GitLab: rebuilt on the git proxy",
      "One meeting, counted once",
      "Grant admin from the person window",
      "“No data yet” on a new installation",
    ]) {
      expect(release.getByRole("heading", { name: title })).toBeInTheDocument();
    }
    // The section names the area, so entries no longer repeat it as a category
    // label of their own.
    expect(screen.queryByText("Git output")).not.toBeInTheDocument();
  });

  it("names every alert destination", () => {
    renderScreen();
    expect(
      screen.getByText(/posts to Slack, Discord, Telegram or Zulip/)
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
