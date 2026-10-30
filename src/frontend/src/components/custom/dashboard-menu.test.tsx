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
      ["Duplicate…", "Rename…", "Pin", "Add to folder…", "Edit tags…", "Delete…"],
    ],
    [
      "editor",
      ["Duplicate…", "Rename…", "Pin", "Add to folder…", "Edit tags…"],
    ],
    [
      "ownerless",
      ["Duplicate…", "Rename…", "Pin", "Add to folder…", "Edit tags…", "Delete…"],
    ],
    ["viewer", ["Pin", "Add to folder…"]],
  ] as const)("offers a caller with %s access %j", async (access, offered) => {
    await openMenu(access);

    await screen.findByRole("menuitem", { name: "Pin" });
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual(offered);
  });
});

describe("adding a dashboard to a folder", () => {
  async function openAdd(access: DashboardAccess) {
    const user = await openMenu(access);
    await user.click(screen.getByRole("menuitem", { name: "Add to folder…" }));
    const dialog = await screen.findByRole("dialog");

    return { user, dialog };
  }

  async function pick(
    user: ReturnType<typeof userEvent.setup>,
    dialog: HTMLElement,
    name: string
  ) {
    await user.click(await within(dialog).findByRole("combobox", { name: "Folder" }));
    await user.click(await screen.findByRole("option", { name }));
  }

  it.each([
    ["owner", { folders: [] }, ["Mine", "Edited"]],
    ["viewer", { folders: [] }, ["Mine", "Edited"]],
    ["ownerless", { folders: [] }, ["Legacy"]],
    ["owner", { folders: [{ id: "Mine", name: "Mine" }] }, ["Edited"]],
  ] as const)(
    "offers a %s dashboard held in %j only the folders %j",
    async (access, held, offered) => {
      vi.mocked(customClient.fetchFolders).mockResolvedValue({
        folders: FOLDERS,
        unfiled: 1,
      });
      reading({ access, folders: [...held.folders] });
      const { user, dialog } = await openAdd(access);

      await user.click(
        await within(dialog).findByRole("combobox", { name: "Folder" })
      );
      const options = await screen.findAllByRole("option");
      expect(options.map((option) => option.textContent)).toEqual(offered);
    }
  );

  it("says so when there is no folder to add it to", async () => {
    const { dialog } = await openAdd("owner");

    expect(
      await within(dialog).findByText("No folders to add it to.")
    ).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Add" })).toBeDisabled();
  });

  it("adds it to the picked folder", async () => {
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: FOLDERS,
      unfiled: 1,
    });
    const { user, dialog } = await openAdd("owner");

    await pick(user, dialog, "Edited");
    await user.click(within(dialog).getByRole("button", { name: "Add" }));

    expect(customClient.addToFolder).toHaveBeenCalledWith("Edited", "dash-1");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it.each([
    ["carries, into a shared folder", true, [{ grantee: "everyone" }], true],
    ["carries, into a folder shared with no one", true, [], false],
    ["only files, into a shared folder", false, [{ grantee: "everyone" }], false],
  ] as const)(
    "warns about the folder's people only when the entry %s",
    async (_case, carries, grants, warned) => {
      vi.mocked(customClient.fetchFolders).mockResolvedValue({
        folders: FOLDERS,
        unfiled: 1,
      });
      vi.mocked(customClient.fetchGrants).mockResolvedValue(
        grants.map((grant) => ({
          ...grant,
          level: "viewer",
          granted_by: "p-1",
          created_at: "2026-09-01T00:00:00Z",
        }))
      );
      reading({ carries });
      const { user, dialog } = await openAdd("owner");

      await pick(user, dialog, "Mine");

      const notice = "People with access to Mine will see this dashboard.";
      if (warned) {
        expect(await within(dialog).findByText(notice)).toBeInTheDocument();
      } else {
        await waitFor(() =>
          expect(within(dialog).getByRole("button", { name: "Add" })).toBeEnabled()
        );
        if (carries) {
          await waitFor(() =>
            expect(customClient.fetchGrants).toHaveBeenCalledWith({
              kind: "folders",
              id: "Mine",
            })
          );
          await act(async () => {
            await vi.mocked(customClient.fetchGrants).mock.results[0]?.value;
          });
        }
        expect(within(dialog).queryByText(notice), `case: ${_case}`).toBeNull();
      }
    }
  );
});

describe("adding a dashboard to a folder whose people cannot be read", () => {
  it("warns anyway when the entry carries", async () => {
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: FOLDERS,
      unfiled: 1,
    });
    vi.mocked(customClient.fetchGrants).mockRejectedValue(
      new customClient.CustomApiError(403, null)
    );
    const user = await openMenu("owner");
    await user.click(screen.getByRole("menuitem", { name: "Add to folder…" }));
    const dialog = await screen.findByRole("dialog");

    await user.click(
      await within(dialog).findByRole("combobox", { name: "Folder" })
    );
    await user.click(await screen.findByRole("option", { name: "Mine" }));

    expect(
      await within(dialog).findByText(
        "People with access to Mine will see this dashboard."
      )
    ).toBeInTheDocument();
  });
});
