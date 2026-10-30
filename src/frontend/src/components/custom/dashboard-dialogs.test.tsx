vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchPins: vi.fn(),
    fetchFolders: vi.fn(),
    fetchTags: vi.fn(),
    fetchDashboards: vi.fn(),
    renameDashboard: vi.fn(),
    duplicateDashboard: vi.fn(),
    deleteDefinition: vi.fn(),
  };
});

import {
  QueryClient,
  QueryClientProvider,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";
import {
  definitionPagesQuery,
  foldersQuery,
  pinsQuery,
  tagsQuery,
} from "@/queries/custom";

import { DashboardMenu } from "./dashboard-menu";

const { CustomApiError } = customClient;

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(customClient.fetchPins).mockResolvedValue([
    { id: "dash-1", name: "delivery" },
  ]);
  vi.mocked(customClient.fetchFolders).mockResolvedValue({
    folders: [],
    unfiled: 1,
  });
  vi.mocked(customClient.fetchTags).mockResolvedValue({ tags: [] });
  vi.mocked(customClient.fetchDashboards).mockResolvedValue({
    items: [
      {
        id: "dash-1",
        name: "delivery",
        owner_person_id: null,
        access: "ownerless",
        updated_at: "2026-09-01T00:00:00Z",
        visibility: "private",
      },
    ],
    total: 1,
  });
});

function Lists() {
  useInfiniteQuery(definitionPagesQuery("dashboards"));
  useQuery(foldersQuery());
  useQuery(tagsQuery());
  useQuery(pinsQuery());
  return null;
}

const reads = () => ({
  names: vi.mocked(customClient.fetchDashboards).mock.calls.length,
  folders: vi.mocked(customClient.fetchFolders).mock.calls.length,
  tags: vi.mocked(customClient.fetchTags).mock.calls.length,
  pins: vi.mocked(customClient.fetchPins).mock.calls.length,
});

async function expectListsReadAgain(before: ReturnType<typeof reads>) {
  await waitFor(() => {
    const now = reads();
    expect(now.names).toBeGreaterThan(before.names);
    expect(now.folders).toBeGreaterThan(before.folders);
    expect(now.tags).toBeGreaterThan(before.tags);
    expect(now.pins).toBeGreaterThan(before.pins);
  });
}

async function openCardMenu(name = "delivery") {
  const user = userEvent.setup();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <Lists />
      <DashboardMenu
        id="dash-1"
        name={name}
        title="Delivery"
        access="owner"
      />
    </QueryClientProvider>
  );
  await waitFor(() => expect(reads().pins).toBe(1));

  await user.click(screen.getByRole("button", { name: `More for ${name}` }));
  await screen.findByRole("menu");

  return user;
}

async function choose(user: ReturnType<typeof userEvent.setup>, item: string) {
  await user.click(await screen.findByRole("menuitem", { name: item }));
  return screen.findByRole("dialog");
}

describe("the card's ··· menu", () => {
  it("offers pin, rename and duplicate, then filing and tags, then delete", async () => {
    await openCardMenu();

    await screen.findByRole("menuitem", { name: "Unpin" });
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual([
      "Unpin",
      "Rename…",
      "Duplicate…",
      "Add to folder…",
      "Edit tags…",
      "Delete…",
    ]);
  });
});

describe("renaming from the card", () => {
  it("renames the dashboard by its id, closes, and reads the lists again", async () => {
    vi.mocked(customClient.renameDashboard).mockResolvedValue(undefined);
    const user = await openCardMenu();

    const dialog = await choose(user, "Rename…");
    const field = within(dialog).getByRole("textbox", {
      name: "Dashboard name",
    });
    expect(field).toHaveValue("Delivery");
    await user.clear(field);
    await user.type(field, "shipping");
    const before = reads();
    await user.click(within(dialog).getByRole("button", { name: "Rename" }));

    expect(customClient.renameDashboard).toHaveBeenCalledWith(
      "dash-1",
      "shipping"
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await expectListsReadAgain(before);
  });

  it("closes without asking when the name is unchanged", async () => {
    const user = await openCardMenu();

    const dialog = await choose(user, "Rename…");
    await user.click(within(dialog).getByRole("button", { name: "Rename" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(customClient.renameDashboard).not.toHaveBeenCalled();
  });

  it("says why a name was refused, keeps the field, and reads the lists again", async () => {
    vi.mocked(customClient.renameDashboard).mockRejectedValue(
      new CustomApiError(400, { detail: "name must be 1 to 128 characters" })
    );
    const user = await openCardMenu();

    const dialog = await choose(user, "Rename…");
    const field = within(dialog).getByRole("textbox", {
      name: "Dashboard name",
    });
    await user.clear(field);
    await user.type(field, "hiring");
    const before = reads();
    await user.keyboard("{Enter}");

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "name must be 1 to 128 characters"
    );
    expect(field).toHaveValue("hiring");
    await expectListsReadAgain(before);
  });

  it("forgets what was typed when the rename is cancelled", async () => {
    const user = await openCardMenu();

    let dialog = await choose(user, "Rename…");
    await user.type(
      within(dialog).getByRole("textbox", { name: "Dashboard name" }),
      "-old"
    );
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    await user.click(screen.getByRole("button", { name: "More for delivery" }));
    dialog = await choose(user, "Rename…");

    expect(
      within(dialog).getByRole("textbox", { name: "Dashboard name" })
    ).toHaveValue("Delivery");
    expect(customClient.renameDashboard).not.toHaveBeenCalled();
  });
});

describe("duplicating from the card", () => {
  it("offers the name with copy, duplicates by id, closes, and reads the lists again", async () => {
    vi.mocked(customClient.duplicateDashboard).mockResolvedValue("dash-2");
    const user = await openCardMenu();

    const dialog = await choose(user, "Duplicate…");
    expect(
      within(dialog).getByRole("textbox", { name: "Dashboard name" })
    ).toHaveValue("delivery copy");
    const before = reads();
    await user.click(within(dialog).getByRole("button", { name: "Duplicate" }));

    expect(customClient.duplicateDashboard).toHaveBeenCalledWith(
      "dash-1",
      "delivery copy"
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await expectListsReadAgain(before);
  });

  it("offers a copy of a name at the limit cut to fit, still marked as a copy", async () => {
    const longest = "d".repeat(customClient.DEFINITION_NAME_MAX);
    const user = await openCardMenu(longest);

    const dialog = await choose(user, "Duplicate…");
    const offered = within(dialog).getByRole("textbox", {
      name: "Dashboard name",
    });

    expect(offered).toHaveValue(
      `${"d".repeat(customClient.DEFINITION_NAME_MAX - " copy".length)} copy`
    );
  });

  it("will not duplicate under an empty name", async () => {
    const user = await openCardMenu();

    const dialog = await choose(user, "Duplicate…");
    await user.clear(
      within(dialog).getByRole("textbox", { name: "Dashboard name" })
    );

    expect(
      within(dialog).getByRole("button", { name: "Duplicate" })
    ).toBeDisabled();
  });

  it("says why a copy was refused, and reads the lists again", async () => {
    vi.mocked(customClient.duplicateDashboard).mockRejectedValue(
      new CustomApiError(403, { detail: "only the owner may change it" })
    );
    const user = await openCardMenu();

    const dialog = await choose(user, "Duplicate…");
    const before = reads();
    await user.click(within(dialog).getByRole("button", { name: "Duplicate" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "only the owner may change it"
    );
    await expectListsReadAgain(before);
  });
});

describe("deleting from the card", () => {
  it("asks by title, deletes, closes, and reads the lists and pins again", async () => {
    vi.mocked(customClient.deleteDefinition).mockResolvedValue(undefined);
    const user = await openCardMenu();

    const dialog = await choose(user, "Delete…");
    expect(dialog).toHaveAccessibleName("Delete Delivery?");
    expect(customClient.deleteDefinition).not.toHaveBeenCalled();
    const before = reads();
    await user.click(
      within(dialog).getByRole("button", { name: "Delete dashboard" })
    );

    expect(customClient.deleteDefinition).toHaveBeenCalledWith(
      "dashboards",
      "dash-1"
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await expectListsReadAgain(before);
  });

  it("says why a delete was refused, and reads the lists again", async () => {
    vi.mocked(customClient.deleteDefinition).mockRejectedValue(
      new CustomApiError(404, { detail: "no dashboard named delivery" })
    );
    const user = await openCardMenu();

    const dialog = await choose(user, "Delete…");
    const before = reads();
    await user.click(
      within(dialog).getByRole("button", { name: "Delete dashboard" })
    );

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "no dashboard named delivery"
    );
    await expectListsReadAgain(before);
  });

  it("keeps the dashboard when the delete is cancelled", async () => {
    const user = await openCardMenu();

    const dialog = await choose(user, "Delete…");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(customClient.deleteDefinition).not.toHaveBeenCalled();
  });
});
