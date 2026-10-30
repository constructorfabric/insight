vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/api/custom-client");
const role = vi.hoisted(() => ({ isAdmin: true }));
vi.mock("@/queries/identity-me", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-me")>()),
  useIsAdmin: () => ({
    isAdmin: role.isAdmin,
    isPending: false,
    isError: false,
    retry: () => {},
  }),
}));

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import type { DashboardAccess, DashboardListed } from "@/api/custom-client";
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

function item(
  name: string,
  access: DashboardAccess = "ownerless"
): DashboardListed {
  return {
    id: name,
    name,
    owner_person_id: null,
    access,
    updated_at: "2026-09-01T00:00:00Z",
    visibility: "private" as const,
  };
}

function listing(...names: string[]) {
  return { items: names.map((name) => item(name)), total: names.length };
}

const READ = {
  name: "delivery",
  ownerPersonId: null,
  ownerName: null,
  visibility: null,
  access: "ownerless" as const,
  folders: [],
  carries: true,
};

beforeEach(() => {
  vi.resetAllMocks();
  role.isAdmin = true;
  portalRouter.reset();
  vi.mocked(customClient.fetchOrphanedDashboards).mockResolvedValue({
    items: [],
    total: 0,
  });
});

describe("/portal/custom", () => {
  it("lists every dashboard as a link", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("engineering", "delivery"));
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(
      new Error("no title today")
    );

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("link", { name: "engineering" })
    ).toHaveAttribute("href", "/portal/custom/engineering");
    expect(
      await screen.findByRole("link", { name: "delivery" })
    ).toBeInTheDocument();
  });

  it("titles a card by the dashboard's title alone, not its name", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("lines_of_code_dashboard"));
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({ ...READ, body: { title: "Lines of Code", widgets: [] }, tags: [] });

    render(<Component />, { wrapper });

    expect(await screen.findByText("Lines of Code")).toHaveAttribute("title", "Lines of Code");
    expect(screen.queryByText("lines_of_code_dashboard")).toBeNull();
  });

  it("says so when there are no dashboards yet", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing());

    render(<Component />, { wrapper });

    expect(await screen.findByText("No dashboards yet")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "New dashboard" })).toHaveLength(2);
  });

  it("offers a member nothing to create, and says nothing is shared yet", async () => {
    role.isAdmin = false;
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing());

    render(<Component />, { wrapper });

    expect(
      await screen.findByText("No shared dashboards")
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "New dashboard" })).toBeNull();
    expect(screen.queryByText(/ask the assistant/i)).toBeNull();
  });

  it("shows a loading state before the list resolves", () => {
    vi.mocked(customClient.fetchDashboards).mockReturnValue(
      new Promise(() => {})
    );

    render(<Component />, { wrapper });

    expect(
      screen.getByRole("status", { name: /loading/i })
    ).toBeInTheDocument();
  });

  it("shows a retryable error state when the list fails to load", async () => {
    vi.mocked(customClient.fetchDashboards).mockRejectedValue(
      new Error("network down")
    );

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: /retry/i })
    ).toBeInTheDocument();
  });
});

describe("/portal/custom in a folder", () => {
  const PLATFORM = {
    id: "f1",
    name: "Platform",
    owner_person_id: null,
    access: "ownerless" as const,
    dashboards: 1,
  };

  beforeEach(() => {
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: [PLATFORM],
      unfiled: 2,
    });
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(new Error("untitled"));
  });

  const askedFor = () =>
    vi.mocked(customClient.fetchDashboards).mock.calls.map(([page]) => page?.folder);

  it("lists one folder's dashboards and names it in the heading", async () => {
    portalRouter.set({ folder: "f1" });
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("delivery"));

    render(<Component />, { wrapper });

    expect(await screen.findByRole("heading", { level: 1, name: "Platform" })).toBeInTheDocument();
    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual(["f1"]);
  });

  it("lists the unfiled under a heading of their own", async () => {
    portalRouter.set({ folder: "unfiled" });
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("hiring"));

    render(<Component />, { wrapper });

    expect(await screen.findByRole("heading", { level: 1, name: "Unfiled" })).toBeInTheDocument();
    await screen.findByRole("link", { name: "hiring" });
    expect(askedFor()).toEqual(["unfiled"]);
  });

  it("shows every dashboard for a folder that is not there, and says so", async () => {
    portalRouter.set({ folder: "gone" });
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("delivery"));

    render(<Component />, { wrapper });

    expect(await screen.findByText(/that folder no longer exists/i)).toBeInTheDocument();
    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toContain(undefined);
    expect(askedFor()).not.toContain("gone");
    expect(screen.getByRole("heading", { level: 1, name: "Dashboards" })).toBeInTheDocument();
  });

  it("says an empty folder is empty rather than that nothing exists", async () => {
    portalRouter.set({ folder: "f1" });
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing());

    render(<Component />, { wrapper });

    expect(
      await screen.findByText("This folder is empty"),
    ).toBeInTheDocument();
    expect(screen.queryByText("No dashboards yet")).toBeNull();
  });

  it("says every dashboard is filed when Unfiled is empty", async () => {
    portalRouter.set({ folder: "unfiled" });
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing());

    render(<Component />, { wrapper });

    expect(await screen.findByText("Every dashboard is in a folder")).toBeInTheDocument();
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
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("delivery"));
  });

  const askedFor = () =>
    vi
      .mocked(customClient.fetchDashboards)
      .mock.calls.filter(([page]) => page?.owned === undefined)
      .map(([page]) => ({ folder: page?.folder, tags: page?.tags }));

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

  it("shows each picked tag beside the filter, and drops one from there", async () => {
    portalRouter.set({ tag: ["Ops", "Platform"] });
    const user = userEvent.setup();

    render(<Component />, { wrapper });
    const pickedList = await screen.findByRole("list", { name: "Picked tags" });
    expect(
      within(pickedList).getAllByRole("listitem").map((item) => item.textContent)
    ).toEqual(["Ops", "Platform"]);

    await user.click(screen.getByRole("button", { name: "Remove tag Ops" }));

    expect(portalRouter.search.tag).toEqual(["Platform"]);
  });

  it("clears every pick, and offers no clearing while none is picked", async () => {
    render(<Component />, { wrapper });
    await openFilter();
    expect(
      screen.getByRole("menuitem", { name: "Clear tags" })
    ).toHaveAttribute("aria-disabled", "true");
    await userEvent.keyboard("{Escape}");

    portalRouter.set({ tag: ["Ops"] });
    const user = await openFilter("Tags · 1");
    await user.click(screen.getByRole("menuitem", { name: "Clear tags" }));

    expect(portalRouter.search.tag).toBeUndefined();
    expect(
      await screen.findByRole("button", { name: "Tags" })
    ).toBeInTheDocument();
  });

  it("combines the picked tags with the folder", async () => {
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: [
        {
          id: "f1",
          name: "Platform",
          owner_person_id: null,
          access: "ownerless",
          dashboards: 1,
        },
      ],
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
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing());
    portalRouter.set({ tag: ["Ops"] });
    const user = userEvent.setup();

    render(<Component />, { wrapper });
    expect(
      await screen.findByText("No dashboards with these tags")
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clear filters" }));

    expect(portalRouter.search.tag).toBeUndefined();
  });
});

describe("/portal/custom cards with tags", () => {
  function carrying(tags: string[]) {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("delivery"));
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({ ...READ, body: { title: "Delivery", widgets: [] }, tags });
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

  it("shows every tag the row has room for", async () => {
    carrying(["Hiring", "Ops", "Platform", "Quality", "Release"]);

    render(<Component />, { wrapper });

    expect(await chips()).toEqual(["Hiring", "Ops", "Platform", "Quality", "Release"]);
  });

  it("counts the tags a narrow row has no room for and reads them out", async () => {
    const CHIP_WIDTH = 60;
    const ROW_WIDTH = 150;
    const offset = vi
      .spyOn(HTMLElement.prototype, "offsetWidth", "get")
      .mockReturnValue(CHIP_WIDTH);
    const client = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockImplementation(function (this: HTMLElement) {
        return this.tagName === "UL" ? ROW_WIDTH : 0;
      });
    onTestFinished(() => {
      offset.mockRestore();
      client.mockRestore();
    });
    carrying(["Hiring", "Ops", "Platform", "Quality", "Release"]);

    render(<Component />, { wrapper });

    expect(
      await screen.findByText("4 more: Ops, Platform, Quality, Release")
    ).toBeInTheDocument();
    expect(screen.getByText("+4").parentElement).toHaveAttribute(
      "title",
      "Ops, Platform, Quality, Release"
    );
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
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("delivery"));
    vi.mocked(customClient.fetchTags).mockResolvedValue({
      tags: [{ name: "Ops", dashboards: 1 }],
    });
    vi.mocked(customClient.fetchDashboardRead)
      .mockResolvedValueOnce({ ...READ, body: DELIVERY, tags: ["Ops"] })
      .mockResolvedValue({ ...READ, body: DELIVERY, tags: ["Ops", "Hiring"] });
  });

  const calls = () => ({
    tags: vi.mocked(customClient.fetchTags).mock.calls.length,
    lists: vi.mocked(customClient.fetchDashboards).mock.calls.length,
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

describe("/portal/custom narrowed by ownership", () => {
  beforeEach(() => {
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(
      new Error("untitled")
    );
  });

  const askedFor = () =>
    vi.mocked(customClient.fetchDashboards).mock.calls.map(([page]) => page?.owned);

  it.each([
    ["me", "My dashboards", "No dashboards yet"],
    ["shared", "Shared with me", "No shared dashboards"],
  ] as const)(
    "asks for owned=%s, marks the %s chip current, and says %s when it is empty",
    async (owned, chip, empty) => {
      portalRouter.set({ owned });
      vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing());

      render(<Component />, { wrapper });

      expect(
        await screen.findByRole("heading", { level: 1, name: "Dashboards" })
      ).toBeInTheDocument();
      expect(await screen.findByText(empty)).toBeInTheDocument();
      const browse = screen.getByRole("navigation", { name: "Browse dashboards" });
      expect(
        within(browse).getByRole("link", { name: (name) => name.startsWith(chip) })
      ).toHaveAttribute("aria-current", "page");
      expect(askedFor()).toContain(owned);
    }
  );

  it("marks All current when the link names no ownership", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("delivery"));

    render(<Component />, { wrapper });

    await screen.findByRole("link", { name: "delivery" });
    const browse = screen.getByRole("navigation", { name: "Browse dashboards" });
    expect(within(browse).getByRole("link", { name: /^All/ })).toHaveAttribute(
      "aria-current",
      "page"
    );
    expect(askedFor()).toContain(undefined);
  });

  it("names the owner on a card that has one", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue({
      items: [
        {
          id: "0190aaaa-0000-7000-8000-000000000001",
          name: "delivery",
          owner_person_id: "01900000-0000-7000-8000-00000000000a",
          owner_name: "Jane Doe",
          updated_at: "2026-09-01T00:00:00Z",
          visibility: "private" as const,
          access: "viewer",
        },
      ],
      total: 1,
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Open delivery" })
    ).toHaveAttribute(
      "href",
      "/portal/custom/0190aaaa-0000-7000-8000-000000000001"
    );
  });
});

describe("/portal/custom orphaned dashboards", () => {
  const ORPHANED = {
    id: "release",
    name: "Release notes",
    owner_person_id: "01900000-0000-7000-8000-0000000000e0",
    updated_at: "2026-09-01T00:00:00.000Z",
  };

  beforeEach(() => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(listing("delivery"));
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(
      new Error("untitled")
    );
    vi.mocked(customClient.fetchTags).mockResolvedValue({
      tags: [{ name: "Ops", dashboards: 1 }],
    });
    vi.mocked(customClient.fetchOrphanedDashboards).mockResolvedValue({
      items: [ORPHANED],
      total: 1,
    });
  });

  it("follows the unfiltered list for an administrator", async () => {
    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("heading", { level: 2, name: "Orphaned" })
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Transfer Release notes" })
    ).toBeInTheDocument();
  });

  it("is never asked for by a member", async () => {
    role.isAdmin = false;
    render(<Component />, { wrapper });

    expect(await screen.findByRole("link", { name: "delivery" })).toBeInTheDocument();
    expect(customClient.fetchOrphanedDashboards).not.toHaveBeenCalled();
  });

  it.each([
    { folder: "unfiled" },
    { owned: "me" as const },
    { owned: "shared" as const },
    { tag: ["Ops"] },
  ])("stays off a narrowed list: %o", async (search) => {
    portalRouter.set(search);
    render(<Component />, { wrapper });

    expect(await screen.findByRole("link", { name: "delivery" })).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Orphaned" }),
      `should hide under ${JSON.stringify(search)}`
    ).toBeNull();
    expect(customClient.fetchOrphanedDashboards).not.toHaveBeenCalled();
  });
});
