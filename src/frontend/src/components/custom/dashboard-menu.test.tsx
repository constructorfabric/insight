vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchPins: vi.fn(),
    pinDashboard: vi.fn(),
    unpinDashboard: vi.fn(),
    fetchFolders: vi.fn(),
    fetchDashboardFolder: vi.fn(),
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
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";
import type { DashboardAccess, FolderSummary } from "@/api/custom-client";
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
  vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue(null);
});

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

const FOLDERS: FolderSummary[] = [
  {
    id: "f-mine",
    name: "Mine",
    owner_person_id: "01900000-0000-7000-8000-00000000000a",
    dashboards: 2,
  },
  { id: "f-shared", name: "Everyone", owner_person_id: null, dashboards: 1 },
];

describe("what the ··· menu offers by access", () => {
  it.each([
    [
      "owner",
      ["Pin", "Rename…", "Duplicate…", "New folder…", "Edit tags…", "Delete…"],
    ],
    [
      "ownerless",
      ["Pin", "Rename…", "Duplicate…", "Edit tags…", "Delete…"],
    ],
    ["viewer", ["Pin", "New folder…"]],
  ] as const)("offers a caller with %s access %j", async (access, offered) => {
    await openMenu(access);

    await screen.findByRole("menuitem", { name: "Pin" });
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual(offered);
  });

  it.each([
    ["owner", ["Mine", "Unfiled"]],
    ["viewer", ["Mine", "Unfiled"]],
    ["ownerless", ["Everyone", "Unfiled"]],
  ] as const)(
    "files a dashboard with %s access only in %j",
    async (access, folders) => {
      vi.mocked(customClient.fetchFolders).mockResolvedValue({
        folders: FOLDERS,
        unfiled: 1,
      });

      await openMenu(access);

      await screen.findByRole("menuitemradio", { name: folders[0] });
      expect(
        screen.getAllByRole("menuitemradio").map((item) => item.textContent)
      ).toEqual(folders);
    }
  );
});
