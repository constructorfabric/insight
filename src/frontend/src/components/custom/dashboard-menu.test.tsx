vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchPins: vi.fn(),
    pinDashboard: vi.fn(),
    unpinDashboard: vi.fn(),
    fetchFolders: vi.fn(),
    fetchDashboardRead: vi.fn(),
    fetchGrants: vi.fn(),
    addToFolder: vi.fn(),
    removeFromFolder: vi.fn(),
  };
});
vi.mock("@/components/ui/sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}));

import {
  QueryClient,
  QueryClientProvider,
  useQuery,
} from "@tanstack/react-query";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";
import type {
  DashboardAccess,
  DashboardRead,
  FolderSummary,
} from "@/api/custom-client";
import { toast } from "@/components/ui/sonner";
import { pinsQuery } from "@/queries/custom";

import { DashboardMenu } from "./dashboard-menu";

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(customClient.fetchPins).mockResolvedValue([]);
  vi.mocked(customClient.pinDashboard).mockResolvedValue(undefined);
  vi.mocked(customClient.unpinDashboard).mockResolvedValue(undefined);
  vi.mocked(customClient.fetchFolders).mockResolvedValue({
    folders: [],
    unfiled: 1,
  });
  vi.mocked(customClient.fetchGrants).mockResolvedValue([]);
  vi.mocked(customClient.addToFolder).mockResolvedValue({ carries: true });
  reading();
});

function reading(read: Partial<DashboardRead> = {}) {
  vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
    name: "delivery",
    body: { title: "Delivery", widgets: [] },
    tags: [],
    ownerPersonId: null,
    ownerName: null,
    visibility: null,
    access: "owner",
    folders: [],
    carries: true,
    ...read,
  });
}

function PinnedPane() {
  useQuery(pinsQuery());
  return null;
}

async function openMenu(access: DashboardAccess = "owner") {
  const user = userEvent.setup();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <PinnedPane />
      <DashboardMenu id="dash-1" name="delivery" access={access} />
    </QueryClientProvider>
  );

  await user.click(screen.getByRole("button", { name: "More for delivery" }));
  await screen.findByRole("menu");

  return user;
}

const pinsRead = () => vi.mocked(customClient.fetchPins).mock.calls.length;

describe("pinning from the ··· menu", () => {
  it("pins a dashboard that is not pinned, and reads the pins again", async () => {
    const user = await openMenu();
    const item = await screen.findByRole("menuitem", { name: "Pin" });
    await waitFor(() => expect(item).not.toHaveAttribute("aria-disabled"));
    const before = pinsRead();

    await user.click(item);

    expect(customClient.pinDashboard).toHaveBeenCalledWith("dash-1");
    await waitFor(() => expect(pinsRead()).toBeGreaterThan(before));
  });

  it("unpins a pinned dashboard", async () => {
    vi.mocked(customClient.fetchPins).mockResolvedValue([
      { id: "dash-0", name: "hiring" },
      { id: "dash-1", name: "delivery" },
    ]);

    const user = await openMenu();
    await user.click(await screen.findByRole("menuitem", { name: "Unpin" }));

    expect(customClient.unpinDashboard).toHaveBeenCalledWith("dash-1");
    expect(customClient.pinDashboard).not.toHaveBeenCalled();
  });

  it("says why a pin was refused, and reads the pins again", async () => {
    vi.mocked(customClient.pinDashboard).mockRejectedValue(
      new customClient.CustomApiError(409, {
        detail: "at most 20 dashboards can be pinned",
      })
    );

    const user = await openMenu();
    const item = await screen.findByRole("menuitem", { name: "Pin" });
    await waitFor(() => expect(item).not.toHaveAttribute("aria-disabled"));
    const before = pinsRead();
    await user.click(item);

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        "at most 20 dashboards can be pinned"
      )
    );
    expect(pinsRead()).toBeGreaterThan(before);
  });
});

const folder = (
  id: string,
  access: FolderSummary["access"]
): FolderSummary => ({
  id,
  name: id,
  owner_person_id:
    access === "ownerless" ? null : "01900000-0000-7000-8000-00000000000a",
  access,
  dashboards: 1,
});

const FOLDERS: FolderSummary[] = [
  folder("Mine", "owner"),
  folder("Edited", "editor"),
  folder("Viewed", "viewer"),
  folder("Legacy", "ownerless"),
];

describe("what the ··· menu offers by access", () => {
  it.each([
    [
      "owner",
      ["Duplicate…", "Rename…", "Pin", "Folders", "Edit tags…", "Delete…"],
    ],
    ["editor", ["Duplicate…", "Rename…", "Pin", "Folders", "Edit tags…"]],
    [
      "ownerless",
      ["Duplicate…", "Rename…", "Pin", "Folders", "Edit tags…", "Delete…"],
    ],
    ["viewer", ["Pin", "Folders"]],
  ] as const)("offers a caller with %s access %j", async (access, offered) => {
    await openMenu(access);

    await screen.findByRole("menuitem", { name: "Pin" });
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual(offered);
  });
});

describe("filing a dashboard from the Folders submenu", () => {
  async function openFolders(access: DashboardAccess = "owner") {
    const user = await openMenu(access);
    await user.click(screen.getByRole("menuitem", { name: "Folders" }));

    return user;
  }

  // WORKAROUND: Base UI leaves the open submenu at `pointer-events: none` in jsdom, so its rows are chosen from the keyboard.
  async function choose(
    user: ReturnType<typeof userEvent.setup>,
    item: HTMLElement
  ) {
    act(() => item.focus());
    await user.keyboard("{Enter}");
  }

  const ticked = () =>
    screen
      .getAllByRole("menuitemcheckbox")
      .map((item) => [item.textContent, item.getAttribute("aria-checked")]);

  beforeEach(() => {
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: FOLDERS,
      unfiled: 1,
    });
    vi.mocked(customClient.removeFromFolder).mockResolvedValue(undefined);
  });

  it("ticks the folders holding it beside the ones it may go in", async () => {
    reading({ folders: [{ id: "Mine", name: "Mine" }] });
    await openFolders();

    await screen.findByRole("menuitemcheckbox", { name: "Mine" });
    expect(ticked()).toEqual([
      ["Mine", "true"],
      ["Edited", "false"],
    ]);
  });

  it("says so when there is no folder to file it in", async () => {
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: [],
      unfiled: 1,
    });
    await openFolders();

    expect(
      await screen.findByRole("menuitem", { name: "No folders yet" })
    ).toHaveAttribute("aria-disabled", "true");
  });

  it("files it in an unticked folder and takes it out of a ticked one", async () => {
    reading({ folders: [{ id: "Mine", name: "Mine" }] });
    const user = await openFolders();

    await choose(user, await screen.findByRole("menuitemcheckbox", { name: "Edited" }));
    await choose(user, screen.getByRole("menuitemcheckbox", { name: "Mine" }));

    await waitFor(() => {
      expect(customClient.addToFolder).toHaveBeenCalledWith("Edited", "dash-1");
      expect(customClient.removeFromFolder).toHaveBeenCalledWith(
        "Mine",
        "dash-1"
      );
    });
  });

  it("ticks a folder as soon as it is picked, before the service answers", async () => {
    vi.mocked(customClient.addToFolder).mockReturnValue(new Promise(() => {}));
    const user = await openFolders();

    const edited = await screen.findByRole("menuitemcheckbox", {
      name: "Edited",
    });
    await choose(user, edited);

    expect(edited).toHaveAttribute("aria-checked", "true");
    expect(
      screen.getByRole("menuitemcheckbox", { name: "Mine" })
    ).not.toHaveAttribute("aria-disabled", "true");
  });

  it("unticks a folder again when the service refuses", async () => {
    vi.mocked(customClient.addToFolder).mockRejectedValue(
      new customClient.CustomApiError(403, null)
    );
    const user = await openFolders();

    const edited = await screen.findByRole("menuitemcheckbox", {
      name: "Edited",
    });
    await choose(user, edited);

    await waitFor(() => expect(toast.error).toHaveBeenCalled());
    expect(edited).toHaveAttribute("aria-checked", "false");
  });

  it.each([
    ["carries, into a shared folder", true, [{ grantee: "everyone" }], true],
    ["carries, into a folder shared with no one", true, [], false],
    ["only files, into a shared folder", false, [{ grantee: "everyone" }], false],
  ] as const)(
    "asks first only when the entry %s",
    async (_case, carries, grants, asked) => {
      vi.mocked(customClient.fetchGrants).mockResolvedValue(
        grants.map((grant) => ({
          ...grant,
          level: "viewer",
          granted_by: "p-1",
          created_at: "2026-09-01T00:00:00Z",
        }))
      );
      reading({ carries });
      const user = await openFolders();

      await choose(
        user,
        await screen.findByRole("menuitemcheckbox", { name: "Mine" })
      );

      if (asked) {
        expect(
          await screen.findByRole("dialog", { name: "Add to Mine?" }),
          `case: ${_case}`
        ).toHaveTextContent(
          "People with access to Mine will see this dashboard."
        );
        expect(customClient.addToFolder).not.toHaveBeenCalled();
      } else {
        await waitFor(() =>
          expect(customClient.addToFolder).toHaveBeenCalledWith("Mine", "dash-1")
        );
        expect(screen.queryByRole("dialog"), `case: ${_case}`).toBeNull();
      }
    }
  );

  it("files it in the shared folder once the reader confirms", async () => {
    vi.mocked(customClient.fetchGrants).mockResolvedValue([
      {
        grantee: "everyone",
        level: "viewer",
        granted_by: "p-1",
        created_at: "2026-09-01T00:00:00Z",
      },
    ]);
    const user = await openFolders();

    await choose(
      user,
      await screen.findByRole("menuitemcheckbox", { name: "Mine" })
    );
    const dialog = await screen.findByRole("dialog", { name: "Add to Mine?" });
    await user.click(within(dialog).getByRole("button", { name: "Add" }));

    await waitFor(() =>
      expect(customClient.addToFolder).toHaveBeenCalledWith("Mine", "dash-1")
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("asks first when the folder's people cannot be read", async () => {
    vi.mocked(customClient.fetchGrants).mockRejectedValue(
      new customClient.CustomApiError(403, null)
    );
    const user = await openFolders();

    await choose(
      user,
      await screen.findByRole("menuitemcheckbox", { name: "Mine" })
    );

    expect(
      await screen.findByRole("dialog", { name: "Add to Mine?" })
    ).toBeInTheDocument();
    expect(customClient.addToFolder).not.toHaveBeenCalled();
  });

  it("opens the new-folder dialog from the submenu", async () => {
    const user = await openFolders();

    await choose(user, await screen.findByRole("menuitem", { name: "New folder…" }));

    expect(
      await screen.findByRole("dialog", { name: "New folder" })
    ).toBeInTheDocument();
  });
});
