vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchPins: vi.fn(),
    fetchFolders: vi.fn(),
    fetchTags: vi.fn(),
    fetchDashboardNames: vi.fn(),
    fetchDashboardFolder: vi.fn(),
    renameDefinition: vi.fn(),
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
  vi.mocked(customClient.fetchPins).mockResolvedValue(["delivery"]);
  vi.mocked(customClient.fetchFolders).mockResolvedValue({
    folders: [],
    unfiled: 1,
  });
  vi.mocked(customClient.fetchTags).mockResolvedValue({ tags: [] });
  vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({
    names: ["delivery"],
    total: 1,
  });
  vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue(null);
});

function Lists() {
  useInfiniteQuery(definitionPagesQuery("dashboards"));
  useQuery(foldersQuery());
  useQuery(tagsQuery());
  useQuery(pinsQuery());
  return null;
}

const reads = () => ({
  names: vi.mocked(customClient.fetchDashboardNames).mock.calls.length,
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

async function openCardMenu() {
  const user = userEvent.setup();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <Lists />
      <DashboardMenu name="delivery" title="Delivery" />
    </QueryClientProvider>
  );
  await waitFor(() => expect(reads().pins).toBe(1));

  await user.click(screen.getByRole("button", { name: "More for delivery" }));
  await screen.findByRole("menu");

  return user;
}

async function choose(user: ReturnType<typeof userEvent.setup>, item: string) {
  await user.click(await screen.findByRole("menuitem", { name: item }));
  return screen.findByRole("dialog");
}

describe("the card's ··· menu", () => {
  it("offers pin, rename and duplicate, then filing, then delete", async () => {
    await openCardMenu();

    await screen.findByRole("menuitem", { name: "Unpin" });
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual([
      "Unpin",
      "Rename…",
      "Duplicate…",
      "New folder…",
      "Edit tags…",
      "Delete…",
    ]);
    expect(
      screen.getByRole("menuitemradio", { name: "Unfiled" })
    ).toBeInTheDocument();
  });
});

describe("renaming from the card", () => {
  it("renames under the new name, closes, and reads the lists again", async () => {
    vi.mocked(customClient.renameDefinition).mockResolvedValue({
      name: "shipping",
      rewritten: [],
    });
    const user = await openCardMenu();

    const dialog = await choose(user, "Rename…");
    const field = within(dialog).getByRole("textbox", {
      name: "Dashboard name",
    });
    expect(field).toHaveValue("delivery");
    await user.clear(field);
    await user.type(field, "shipping");
    const before = reads();
    await user.click(within(dialog).getByRole("button", { name: "Rename" }));

    expect(customClient.renameDefinition).toHaveBeenCalledWith(
      "dashboards",
      "delivery",
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
    expect(customClient.renameDefinition).not.toHaveBeenCalled();
  });

  it("says why a name was refused, keeps the field, and reads the lists again", async () => {
    vi.mocked(customClient.renameDefinition).mockRejectedValue(
      new CustomApiError(409, { detail: "a dashboard named hiring exists" })
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
      "a dashboard named hiring exists"
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
    ).toHaveValue("delivery");
    expect(customClient.renameDefinition).not.toHaveBeenCalled();
  });
});

describe("duplicating from the card", () => {
  it("offers the name with -copy, duplicates, closes, and reads the lists again", async () => {
    vi.mocked(customClient.duplicateDashboard).mockResolvedValue(
      "delivery-copy"
    );
    const user = await openCardMenu();

    const dialog = await choose(user, "Duplicate…");
    expect(
      within(dialog).getByRole("textbox", { name: "Dashboard name" })
    ).toHaveValue("delivery-copy");
    const before = reads();
    await user.click(within(dialog).getByRole("button", { name: "Duplicate" }));

    expect(customClient.duplicateDashboard).toHaveBeenCalledWith(
      "delivery",
      "delivery-copy"
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await expectListsReadAgain(before);
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
      new CustomApiError(409, { detail: "delivery-copy is taken" })
    );
    const user = await openCardMenu();

    const dialog = await choose(user, "Duplicate…");
    const before = reads();
    await user.click(within(dialog).getByRole("button", { name: "Duplicate" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "delivery-copy is taken"
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
      "delivery"
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
