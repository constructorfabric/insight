vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchPins: vi.fn(),
    pinDashboard: vi.fn(),
    unpinDashboard: vi.fn(),
    fetchFolders: vi.fn(),
    renameDashboard: vi.fn(),
    duplicateDashboard: vi.fn(),
    deleteDefinition: vi.fn(),
  };
});
vi.mock("@/components/custom/share-dialog", () => ({
  ShareDialog: ({
    open,
    target,
    ownerPersonId,
  }: {
    open: boolean;
    target: { kind: string; id: string };
    ownerPersonId: string;
  }) =>
    open ? (
      <div role="dialog" aria-label="Share">
        {`${target.kind}/${target.id} ${ownerPersonId}`}
      </div>
    ) : null,
}));

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";
import type { DashboardAccess } from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { DashboardPageActions } from "./dashboard-page-actions";

const { CustomApiError } = customClient;

const ID = "dash-1";
const OWNER = "01900000-0000-7000-8000-00000000000a";

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset(`/portal/custom/${ID}`);
  vi.mocked(customClient.fetchPins).mockResolvedValue([]);
  vi.mocked(customClient.pinDashboard).mockResolvedValue(undefined);
  vi.mocked(customClient.unpinDashboard).mockResolvedValue(undefined);
  vi.mocked(customClient.fetchFolders).mockResolvedValue({
    folders: [],
    unfiled: 1,
  });
  vi.mocked(customClient.renameDashboard).mockResolvedValue(undefined);
  vi.mocked(customClient.duplicateDashboard).mockResolvedValue("dash-2");
  vi.mocked(customClient.deleteDefinition).mockResolvedValue(undefined);
});

function actions(access: DashboardAccess = "owner", carries = access !== "viewer") {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <DashboardPageActions
        id={ID}
        name="delivery"
        title="Delivery"
        access={access}
        ownerPersonId={access === "ownerless" ? null : OWNER}
        carries={carries}
      />
    </QueryClientProvider>
  );
}

async function openMenu(access: DashboardAccess = "owner") {
  const user = userEvent.setup();
  actions(access);

  await user.click(screen.getByRole("button", { name: "More for delivery" }));
  await screen.findByRole("menu");

  return user;
}

async function choose(user: ReturnType<typeof userEvent.setup>, item: string) {
  await user.click(await screen.findByRole("menuitem", { name: item }));
  return screen.findByRole("dialog");
}

describe("<DashboardPageActions>", () => {
  it("links Edit to the dashboard's editor by id", () => {
    actions();

    expect(screen.getByRole("button", { name: "Edit delivery" })).toHaveAttribute(
      "href",
      `/portal/custom/edit/dashboards/${ID}`
    );
  });

  it.each([
    ["owner", true, true, true],
    ["editor", true, true, true],
    ["editor", false, false, true],
    ["ownerless", true, false, true],
    ["viewer", false, false, false],
  ] as const)(
    "a caller with %s access, carrying %s, is offered Share: %s, Edit: %s",
    async (access, carries, share, edit) => {
      const user = userEvent.setup();
      actions(access, carries);

      expect(
        screen.queryByRole("button", { name: "Edit delivery" }) !== null
      ).toBe(edit);
      await user.click(screen.getByRole("button", { name: "More for delivery" }));
      await screen.findByRole("menu");
      expect(screen.queryByRole("menuitem", { name: "Share…" }) !== null).toBe(
        share
      );
    }
  );

  it.each(["owner", "editor"] as const)(
    "opens the dashboard's share dialog for an %s",
    async (access) => {
      const user = await openMenu(access);

      await user.click(await screen.findByRole("menuitem", { name: "Share…" }));

      expect(
        await screen.findByRole("dialog", { name: "Share" })
      ).toHaveTextContent(`dashboards/${ID} ${OWNER}`);
    }
  );

  it("offers the owner duplicate, share, the planned exports, rename, pin, filing, tags and delete", async () => {
    await openMenu();

    await screen.findByRole("menuitem", { name: "Pin" });
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual([
      "Duplicate…",
      "Share…",
      "Export snapshot (PDF)Planned",
      "Schedule report…Planned",
      "Rename…",
      "Pin",
      "Add to folder…",
      "Edit tags…",
      "Delete…",
    ]);
  });

  it("offers an editor everything but delete", async () => {
    await openMenu("editor");

    await screen.findByRole("menuitem", { name: "Pin" });
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual([
      "Duplicate…",
      "Share…",
      "Export snapshot (PDF)Planned",
      "Schedule report…Planned",
      "Rename…",
      "Pin",
      "Add to folder…",
      "Edit tags…",
    ]);
  });

  it("pins the dashboard on screen by its id", async () => {
    const user = await openMenu();

    const item = await screen.findByRole("menuitem", { name: "Pin" });
    await waitFor(() => expect(item).not.toHaveAttribute("aria-disabled"));
    await user.click(item);

    expect(customClient.pinDashboard).toHaveBeenCalledWith(ID);
  });

  it("unpins the dashboard on screen", async () => {
    vi.mocked(customClient.fetchPins).mockResolvedValue([
      { id: ID, name: "delivery" },
    ]);
    const user = await openMenu();

    await user.click(await screen.findByRole("menuitem", { name: "Unpin" }));

    expect(customClient.unpinDashboard).toHaveBeenCalledWith(ID);
  });

  it("stays on the page after a rename, since the id does not change", async () => {
    const user = await openMenu();

    const dialog = await choose(user, "Rename…");
    const field = within(dialog).getByRole("textbox", {
      name: "Dashboard name",
    });
    await user.clear(field);
    await user.type(field, "shipping{Enter}");

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(customClient.renameDashboard).toHaveBeenCalledWith(ID, "shipping");
    expect(portalRouter.navigations).toHaveLength(0);
  });

  it("lands on the copy's id after a duplicate", async () => {
    const user = await openMenu();

    const dialog = await choose(user, "Duplicate…");
    await user.click(within(dialog).getByRole("button", { name: "Duplicate" }));

    await waitFor(() =>
      expect(portalRouter.pathname).toBe("/portal/custom/dash-2")
    );
  });

  it("lands on the dashboards list after a delete", async () => {
    const user = await openMenu();

    const dialog = await choose(user, "Delete…");
    await user.click(
      within(dialog).getByRole("button", { name: "Delete dashboard" })
    );

    await waitFor(() => expect(portalRouter.pathname).toBe("/portal/custom"));
  });

  it.each([
    [
      "Rename…",
      "Rename",
      customClient.renameDashboard,
      "only the owner may change it",
    ],
    [
      "Duplicate…",
      "Duplicate",
      customClient.duplicateDashboard,
      "only the owner may change it",
    ],
    [
      "Delete…",
      "Delete dashboard",
      customClient.deleteDefinition,
      "a widget still draws it",
    ],
  ])(
    "stays on the page and says why when %s is refused",
    async (item, confirm, call, said) => {
      vi.mocked(call).mockRejectedValue(
        new CustomApiError(409, { detail: said })
      );
      const user = await openMenu();

      const dialog = await choose(user, item);
      if (item === "Rename…") {
        const field = within(dialog).getByRole("textbox", {
          name: "Dashboard name",
        });
        await user.clear(field);
        await user.type(field, "shipping");
      }
      await user.click(within(dialog).getByRole("button", { name: confirm }));

      expect(await within(dialog).findByRole("alert")).toHaveTextContent(said);
      expect(portalRouter.navigations).toHaveLength(0);
    }
  );
});
