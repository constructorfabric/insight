vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/api/custom-client");

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.index";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset();
});

describe("/portal/custom", () => {
  it("lists every dashboard as a link", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [
      "engineering",
      "delivery",
    ], total: 2 });
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(
      new Error("no title today")
    );

    render(<Component />, { wrapper });

    // The identifier stands in until a title arrives, so the link is still
    // reachable when a definition cannot be read.
    expect(
      await screen.findByRole("link", { name: "engineering" })
    ).toHaveAttribute("href", "/portal/custom/engineering");
    expect(
      await screen.findByRole("link", { name: "delivery" })
    ).toBeInTheDocument();
  });

  it("titles a card by the dashboard, keeping the identifier beneath it", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [
      "lines_of_code_dashboard",
    ], total: 1 });
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      body: { title: "Lines of Code", widgets: [] },
      tags: [],
    });

    render(<Component />, { wrapper });

    // The reader picks a dashboard by its name, not by its slug.
    expect(await screen.findByText("Lines of Code")).toBeInTheDocument();
    expect(
      await screen.findByText("lines_of_code_dashboard")
    ).toBeInTheDocument();
  });

  it("says so when there are no dashboards yet", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [], total: 0 });

    render(<Component />, { wrapper });

    expect(await screen.findByText(/no dashboards yet/i)).toBeInTheDocument();
  });

  it("shows a loading state before the list resolves", () => {
    vi.mocked(customClient.fetchDashboardNames).mockReturnValue(
      new Promise(() => {})
    );

    render(<Component />, { wrapper });

    expect(
      screen.getByRole("status", { name: /loading/i })
    ).toBeInTheDocument();
  });

  it("shows a retryable error state when the list fails to load", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockRejectedValue(
      new Error("network down")
    );

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: /retry/i })
    ).toBeInTheDocument();
  });
});

describe("/portal/custom in a folder", () => {
  const PLATFORM = { id: "f1", name: "Platform", dashboards: 1 };

  beforeEach(() => {
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: [PLATFORM],
      unfiled: 2,
    });
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(new Error("untitled"));
  });

  const askedFor = () =>
    vi.mocked(customClient.fetchDashboardNames).mock.calls.map(([page]) => page?.folder);

  it("lists one folder's dashboards and names it in the heading", async () => {
    portalRouter.set({ folder: "f1" });
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["delivery"], total: 1 });

    render(<Component />, { wrapper });

    expect(await screen.findByRole("heading", { level: 1, name: "Platform" })).toBeInTheDocument();
    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual(["f1"]);
  });

  it("lists the unfiled under a heading of their own", async () => {
    portalRouter.set({ folder: "unfiled" });
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["hiring"], total: 1 });

    render(<Component />, { wrapper });

    expect(await screen.findByRole("heading", { level: 1, name: "Unfiled" })).toBeInTheDocument();
    await screen.findByRole("link", { name: "hiring" });
    expect(askedFor()).toEqual(["unfiled"]);
  });

  it("shows every dashboard for a folder that is not there, and says so", async () => {
    portalRouter.set({ folder: "gone" });
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["delivery"], total: 1 });

    render(<Component />, { wrapper });

    expect(await screen.findByText(/that folder no longer exists/i)).toBeInTheDocument();
    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual([undefined]);
    expect(screen.getByRole("heading", { level: 1, name: "Custom" })).toBeInTheDocument();
  });

  it("says how to fill an empty folder rather than that nothing exists", async () => {
    portalRouter.set({ folder: "f1" });
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [], total: 0 });

    render(<Component />, { wrapper });

    expect(
      await screen.findByText("No dashboards in this folder yet. Move one here from its ··· menu."),
    ).toBeInTheDocument();
    expect(screen.queryByText(/no dashboards yet\. describe/i)).toBeNull();
  });

  it("says every dashboard is filed when Unfiled is empty", async () => {
    portalRouter.set({ folder: "unfiled" });
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [], total: 0 });

    render(<Component />, { wrapper });

    expect(await screen.findByText("Every dashboard is in a folder.")).toBeInTheDocument();
  });

  it("checks the folder a dashboard is in", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["delivery"], total: 1 });
    vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue({ id: "f1", name: "Platform" });
    const user = userEvent.setup();

    render(<Component />, { wrapper });
    await user.click(await screen.findByRole("button", { name: "More for delivery" }));

    await waitFor(() =>
      expect(screen.getByRole("menuitemradio", { name: /Platform/ })).toHaveAttribute("aria-checked", "true"),
    );
    expect(screen.getByRole("menuitemradio", { name: /Unfiled/ })).toHaveAttribute("aria-checked", "false");
  });

  it("moves a dashboard out of the folder on screen, and the card leaves", async () => {
    portalRouter.set({ folder: "f1" });
    vi.mocked(customClient.fetchDashboardNames)
      .mockResolvedValueOnce({ names: ["delivery"], total: 1 })
      .mockResolvedValue({ names: [], total: 0 });
    vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue({ id: "f1", name: "Platform" });
    vi.mocked(customClient.moveDashboard).mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(<Component />, { wrapper });
    await user.click(await screen.findByRole("button", { name: "More for delivery" }));
    await user.click(await screen.findByRole("menuitemradio", { name: /Unfiled/ }));

    expect(customClient.moveDashboard).toHaveBeenCalledWith("delivery", null);
    await waitFor(() => expect(screen.queryByRole("link", { name: "delivery" })).toBeNull());
  });

  it("makes a folder and then moves the dashboard into it", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["delivery"], total: 1 });
    vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue(null);
    vi.mocked(customClient.createFolder).mockResolvedValue({ id: "f9", name: "Hiring" });
    vi.mocked(customClient.moveDashboard).mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(<Component />, { wrapper });
    await user.click(await screen.findByRole("button", { name: "More for delivery" }));
    await user.click(await screen.findByRole("menuitem", { name: /New folder/ }));
    await user.type(await screen.findByRole("textbox", { name: "Folder name" }), "Hiring");
    await user.click(screen.getByRole("button", { name: "Create and move" }));

    await waitFor(() => expect(customClient.moveDashboard).toHaveBeenCalledWith("delivery", "f9"));
    expect(customClient.createFolder).toHaveBeenCalledWith("Hiring");
    expect(vi.mocked(customClient.createFolder).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(customClient.moveDashboard).mock.invocationCallOrder[0],
    );
  });
});

describe("/portal/custom filtered by tag", () => {
  const TAGS = [
    { name: "Ops", dashboards: 2 },
    { name: "Platform", dashboards: 1 },
  ];

  beforeEach(() => {
    vi.mocked(customClient.fetchTags).mockResolvedValue({ tags: TAGS });
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(
      new Error("untitled")
    );
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({
      names: ["delivery"],
      total: 1,
    });
  });

  const askedFor = () =>
    vi
      .mocked(customClient.fetchDashboardNames)
      .mock.calls.map(([page]) => ({ folder: page?.folder, tags: page?.tags }));

  async function openFilter(name = "Tags") {
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name }));
    await screen.findByRole("menu");
    return user;
  }

  it("offers every tag beside the search, none of them picked", async () => {
    render(<Component />, { wrapper });
    await openFilter();

    expect(
      screen.getByRole("menuitemcheckbox", { name: "Ops" })
    ).toHaveAttribute("aria-checked", "false");
    expect(
      screen.getByRole("menuitemcheckbox", { name: "Platform" })
    ).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText("Filter by tag")).toBeInTheDocument();
  });

  it("picks a tag into the URL and narrows the list to it", async () => {
    render(<Component />, { wrapper });
    const user = await openFilter();

    await user.click(screen.getByRole("menuitemcheckbox", { name: "Ops" }));

    expect(portalRouter.search.tag).toEqual(["Ops"]);
    expect(
      await screen.findByRole("button", { name: "Tags · 1" })
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(askedFor()).toContainEqual({ folder: undefined, tags: ["Ops"] })
    );
  });

  it("unpicks a tag and keeps the others", async () => {
    portalRouter.set({ tag: ["Ops", "Platform"] });

    render(<Component />, { wrapper });
    const user = await openFilter("Tags · 2");
    expect(
      screen.getByRole("menuitemcheckbox", { name: "Ops" })
    ).toHaveAttribute("aria-checked", "true");
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Ops" }));

    expect(portalRouter.search.tag).toEqual(["Platform"]);
    expect(
      await screen.findByRole("button", { name: "Tags · 1" })
    ).toBeInTheDocument();
  });

  it("clears every pick, and offers no clearing while none is picked", async () => {
    render(<Component />, { wrapper });
    await openFilter();
    expect(
      screen.getByRole("menuitem", { name: "Clear filters" })
    ).toHaveAttribute("aria-disabled", "true");
    await userEvent.keyboard("{Escape}");

    portalRouter.set({ tag: ["Ops"] });
    const user = await openFilter("Tags · 1");
    await user.click(screen.getByRole("menuitem", { name: "Clear filters" }));

    expect(portalRouter.search.tag).toBeUndefined();
    expect(
      await screen.findByRole("button", { name: "Tags" })
    ).toBeInTheDocument();
  });

  it("combines the picked tags with the folder", async () => {
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: [{ id: "f1", name: "Platform", dashboards: 1 }],
      unfiled: 0,
    });
    portalRouter.set({ folder: "f1", tag: ["Ops", "Platform"] });

    render(<Component />, { wrapper });

    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual([
      { folder: "f1", tags: ["Ops", "Platform"] },
    ]);
  });

  it("drops a tag that no longer exists from the request", async () => {
    portalRouter.set({ tag: ["Ops", "Gone"] });

    render(<Component />, { wrapper });

    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual([{ folder: undefined, tags: ["Ops"] }]);
    expect(screen.getByRole("button", { name: "Tags · 1" })).toBeInTheDocument();
  });

  it("asks for every dashboard when no named tag exists any more", async () => {
    portalRouter.set({ tag: ["Gone"] });

    render(<Component />, { wrapper });

    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual([{ folder: undefined, tags: undefined }]);
  });

  it("asks for a tag under its stored spelling", async () => {
    portalRouter.set({ tag: ["ops"] });

    render(<Component />, { wrapper });

    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual([{ folder: undefined, tags: ["Ops"] }]);
  });

  it("asks with the tags the link names when the tag list cannot be read", async () => {
    vi.mocked(customClient.fetchTags).mockRejectedValue(new Error("down"));
    portalRouter.set({ tag: ["Ops"] });

    render(<Component />, { wrapper });

    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual([{ folder: undefined, tags: ["Ops"] }]);
  });

  it("says when no dashboard has a tag yet", async () => {
    vi.mocked(customClient.fetchTags).mockResolvedValue({ tags: [] });

    render(<Component />, { wrapper });
    await openFilter();

    expect(
      screen.getByRole("menuitem", { name: "No tags yet" })
    ).toHaveAttribute("aria-disabled", "true");
  });

  it("says no dashboard carries the picked tags, and clears them", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({
      names: [],
      total: 0,
    });
    portalRouter.set({ tag: ["Ops"] });
    const user = userEvent.setup();

    render(<Component />, { wrapper });
    expect(
      await screen.findByText("No dashboard carries these tags.")
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clear filters" }));

    expect(portalRouter.search.tag).toBeUndefined();
  });
});

describe("/portal/custom cards with tags", () => {
  function carrying(tags: string[]) {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({
      names: ["delivery"],
      total: 1,
    });
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      body: { title: "Delivery", widgets: [] },
      tags,
    });
  }

  const chips = async () =>
    within(await screen.findByRole("list", { name: "Tags" }))
      .getAllByRole("listitem")
      .map((chip) => chip.textContent);

  it("shows a card's tags under its name, outside its link", async () => {
    carrying(["Ops", "Platform"]);

    render(<Component />, { wrapper });

    expect(await chips()).toEqual(["Ops", "Platform"]);
    expect(
      screen.getByRole("link", { name: /Delivery/ })
    ).not.toHaveTextContent("Ops");
  });

  it("shows three tags and counts the rest", async () => {
    carrying(["Hiring", "Ops", "Platform", "Quality", "Release"]);

    render(<Component />, { wrapper });

    expect(await chips()).toEqual(["Hiring", "Ops", "Platform", "+2"]);
    expect(screen.getByText("+2")).toHaveAttribute("title", "Quality, Release");
  });

  it("shows no tag row on a card without tags", async () => {
    carrying([]);

    render(<Component />, { wrapper });

    await screen.findByText("Delivery");
    expect(screen.queryByRole("list", { name: "Tags" })).toBeNull();
  });
});

describe("/portal/custom after a card's tags are set", () => {
  const DELIVERY = { title: "Delivery", widgets: [] };

  beforeEach(() => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({
      names: ["delivery"],
      total: 1,
    });
    vi.mocked(customClient.fetchTags).mockResolvedValue({
      tags: [{ name: "Ops", dashboards: 1 }],
    });
    vi.mocked(customClient.fetchDashboardRead)
      .mockResolvedValueOnce({ body: DELIVERY, tags: ["Ops"] })
      .mockResolvedValue({ body: DELIVERY, tags: ["Ops", "Hiring"] });
  });

  const calls = () => ({
    tags: vi.mocked(customClient.fetchTags).mock.calls.length,
    lists: vi.mocked(customClient.fetchDashboardNames).mock.calls.length,
  });

  async function addHiring() {
    const user = userEvent.setup();
    render(<Component />, { wrapper });
    await user.click(
      await screen.findByRole("button", { name: "More for delivery" })
    );
    await user.click(await screen.findByRole("menuitem", { name: /tags/ }));
    await user.type(
      await screen.findByRole("textbox", { name: "Add tag" }),
      "Hiring"
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Done" })).toBeEnabled()
    );

    const before = calls();
    await user.click(screen.getByRole("button", { name: "Done" }));
    return before;
  }

  it("shows the new tags on the card and reads the tags and the list again", async () => {
    vi.mocked(customClient.setDashboardTags).mockResolvedValue(undefined);

    const before = await addHiring();

    await waitFor(() =>
      expect(
        within(screen.getByRole("list", { name: "Tags" })).getByText("Hiring")
      ).toBeInTheDocument()
    );
    expect(calls().tags).toBeGreaterThan(before.tags);
    expect(calls().lists).toBeGreaterThan(before.lists);
  });

  it("reads the tags and the list again after a refused save", async () => {
    vi.mocked(customClient.setDashboardTags).mockRejectedValue(
      new Error("down")
    );

    const before = await addHiring();

    expect(
      await screen.findByText("The tags could not be saved.")
    ).toBeInTheDocument();
    await waitFor(() => expect(calls().tags).toBeGreaterThan(before.tags));
    expect(calls().lists).toBeGreaterThan(before.lists);
  });
});
