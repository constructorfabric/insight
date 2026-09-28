vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchTags: vi.fn(),
    fetchDashboardRead: vi.fn(),
    setDashboardTags: vi.fn(),
    fetchFolders: vi.fn(),
    fetchDashboardFolder: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";

import { DashboardMenu } from "./dashboard-menu";

const TAGS = [
  { name: "Ops", dashboards: 2 },
  { name: "Platform", dashboards: 1 },
];

function carrying(tags: string[]) {
  vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
    body: { title: "Delivery", widgets: [] },
    tags,
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(customClient.fetchTags).mockResolvedValue({ tags: TAGS });
  vi.mocked(customClient.fetchFolders).mockResolvedValue({
    folders: [],
    unfiled: 1,
  });
  vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue(null);
  vi.mocked(customClient.setDashboardTags).mockResolvedValue(undefined);
  carrying(["Ops"]);
});

async function openTagsDialog(staleTime = 0) {
  const user = userEvent.setup();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <DashboardMenu name="delivery" />
    </QueryClientProvider>
  );

  await user.click(screen.getByRole("button", { name: "More for delivery" }));
  await user.click(await screen.findByRole("menuitem", { name: /Edit tags/ }));
  const dialog = await screen.findByRole("dialog", { name: "Edit tags" });

  return { user, dialog };
}

const box = (name: string) => screen.getByRole("checkbox", { name });

const choices = (dialog: HTMLElement) =>
  within(dialog)
    .getAllByRole("checkbox")
    .map((one) => one.getAttribute("aria-label") ?? one.textContent);

describe("the tags dialog", () => {
  it("ticks the tags the dashboard carries among every tag", async () => {
    await openTagsDialog();

    await waitFor(() => expect(box("Ops")).toBeChecked());
    expect(box("Platform")).not.toBeChecked();
  });

  it("sends the whole ticked set on Done, then closes", async () => {
    const { user } = await openTagsDialog();

    await user.click(await screen.findByRole("checkbox", { name: "Platform" }));
    await user.click(screen.getByRole("button", { name: "Done" }));

    expect(customClient.setDashboardTags).toHaveBeenCalledWith("delivery", [
      "Ops",
      "Platform",
    ]);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("sends an empty set once every tick is taken off", async () => {
    const { user } = await openTagsDialog();

    await user.click(await screen.findByRole("checkbox", { name: "Ops" }));
    await user.click(screen.getByRole("button", { name: "Done" }));

    expect(customClient.setDashboardTags).toHaveBeenCalledWith("delivery", []);
  });

  it("adds a new name ticked", async () => {
    const { user } = await openTagsDialog();

    await user.type(
      screen.getByRole("textbox", { name: "Add tag" }),
      "Hiring{Enter}"
    );

    expect(box("Hiring")).toBeChecked();
    expect(screen.getByRole("textbox", { name: "Add tag" })).toHaveValue("");
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(customClient.setDashboardTags).toHaveBeenCalledWith("delivery", [
      "Ops",
      "Hiring",
    ]);
  });

  it("ticks the tag a typed name already belongs to, under its stored spelling", async () => {
    const { user, dialog } = await openTagsDialog();
    await screen.findByRole("checkbox", { name: "Platform" });

    await user.type(
      screen.getByRole("textbox", { name: "Add tag" }),
      "platform{Enter}"
    );

    expect(box("Platform")).toBeChecked();
    expect(within(dialog).getAllByRole("checkbox")).toHaveLength(2);
  });

  it("adds nothing for a blank name", async () => {
    const { user, dialog } = await openTagsDialog();
    await screen.findByRole("checkbox", { name: "Platform" });

    await user.type(
      screen.getByRole("textbox", { name: "Add tag" }),
      "   {Enter}"
    );

    expect(choices(dialog)).toHaveLength(2);
  });

  it("takes a name still in the field when Done is pressed", async () => {
    const { user } = await openTagsDialog();
    await screen.findByRole("checkbox", { name: "Ops" });

    await user.type(screen.getByRole("textbox", { name: "Add tag" }), "Hiring");
    await user.click(screen.getByRole("button", { name: "Done" }));

    expect(customClient.setDashboardTags).toHaveBeenCalledWith("delivery", [
      "Ops",
      "Hiring",
    ]);
  });

  it("discards the changes on Cancel", async () => {
    const { user } = await openTagsDialog();

    await user.click(await screen.findByRole("checkbox", { name: "Platform" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    await user.click(screen.getByRole("button", { name: "More for delivery" }));
    await user.click(
      await screen.findByRole("menuitem", { name: /Edit tags/ })
    );
    expect(
      await screen.findByRole("checkbox", { name: "Platform" })
    ).not.toBeChecked();
    expect(customClient.setDashboardTags).not.toHaveBeenCalled();
  });

  it("discards the changes on Escape", async () => {
    const { user } = await openTagsDialog();

    await user.click(await screen.findByRole("checkbox", { name: "Platform" }));
    await user.keyboard("{Escape}");

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(customClient.setDashboardTags).not.toHaveBeenCalled();
  });

  it("refuses an eleventh tag, under the field", async () => {
    const ten = Array.from({ length: 10 }, (_, i) => `Tag ${i + 1}`);
    carrying(ten);
    vi.mocked(customClient.fetchTags).mockResolvedValue({
      tags: [...ten, "Ops"].map((name) => ({ name, dashboards: 1 })),
    });
    const { user } = await openTagsDialog();

    await user.click(await screen.findByRole("checkbox", { name: "Ops" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "A dashboard carries at most 10 tags."
    );
    expect(box("Ops")).not.toBeChecked();

    await user.type(
      screen.getByRole("textbox", { name: "Add tag" }),
      "Hiring{Enter}"
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "A dashboard carries at most 10 tags."
    );
    expect(screen.queryByRole("checkbox", { name: "Hiring" })).toBeNull();
  });

  it("refuses a name longer than 32 characters, under the field", async () => {
    const { user } = await openTagsDialog();
    await screen.findByRole("checkbox", { name: "Ops" });

    await user.type(
      screen.getByRole("textbox", { name: "Add tag" }),
      `${"x".repeat(33)}{Enter}`
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "A tag name is at most 32 characters."
    );
    expect(screen.getAllByRole("checkbox")).toHaveLength(2);
  });

  it("shows what the service said when it refuses the set, and stays open", async () => {
    vi.mocked(customClient.setDashboardTags).mockRejectedValue(
      new customClient.CustomApiError(409, {
        detail: "At most 200 tags can exist.",
      })
    );
    const { user } = await openTagsDialog();

    await user.click(await screen.findByRole("checkbox", { name: "Platform" }));
    await user.click(screen.getByRole("button", { name: "Done" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "At most 200 tags can exist."
    );
    expect(
      screen.getByRole("dialog", { name: "Edit tags" })
    ).toBeInTheDocument();
  });

  it("waits for the dashboard's own tags before it can save", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockReturnValue(
      new Promise(() => {})
    );

    await openTagsDialog();

    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
  });

  it("keeps the dashboard's own tags when a name is typed before they load", async () => {
    type Read = Awaited<ReturnType<typeof customClient.fetchDashboardRead>>;
    let arrive: (read: Read) => void = () => {};
    vi.mocked(customClient.fetchDashboardRead).mockReturnValue(
      new Promise((resolve) => {
        arrive = resolve;
      })
    );

    const { user } = await openTagsDialog();
    await user.type(screen.getByRole("textbox", { name: "Add tag" }), "Qa{Enter}");
    arrive({ body: { title: "Delivery", widgets: [] }, tags: ["Ops"] });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Done" })).toBeEnabled()
    );
    await user.click(screen.getByRole("button", { name: "Done" }));

    expect(customClient.setDashboardTags).toHaveBeenCalledWith(
      "delivery",
      expect.arrayContaining(["Ops"])
    );
  });

  it("reads the dashboard's tags afresh each time it opens", async () => {
    const { user } = await openTagsDialog(60 * 60 * 1000);
    await waitFor(() => expect(box("Ops")).toBeChecked());
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    carrying(["Ops", "Platform"]);
    await user.click(screen.getByRole("button", { name: "More for delivery" }));
    await user.click(await screen.findByRole("menuitem", { name: /Edit tags/ }));
    await waitFor(() => expect(box("Platform")).toBeChecked());
    await user.click(screen.getByRole("button", { name: "Done" }));

    expect(customClient.setDashboardTags).toHaveBeenCalledWith("delivery", [
      "Ops",
      "Platform",
    ]);
  });

  it("says when the dashboard's own tags cannot be read, and cannot save", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(
      new Error("down")
    );

    await openTagsDialog();

    expect(
      await screen.findByText("The dashboard's tags could not be read.")
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
  });
});
