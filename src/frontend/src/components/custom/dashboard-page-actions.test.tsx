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
    fetchDashboardFolder: vi.fn(),
    renameDefinition: vi.fn(),
    duplicateDashboard: vi.fn(),
    deleteDefinition: vi.fn(),
  };
});
vi.mock("@/components/custom/share-dashboard", () => ({
  ShareDashboard: ({
    open,
    ownerPersonId,
  }: {
    open: boolean;
    ownerPersonId: string;
  }) =>
    open ? (
      <div role="dialog" aria-label="Share">
        {ownerPersonId}
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
  vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue(null);
  vi.mocked(customClient.renameDefinition).mockResolvedValue(undefined);
  vi.mocked(customClient.duplicateDashboard).mockResolvedValue("dash-2");
  vi.mocked(customClient.deleteDefinition).mockResolvedValue(undefined);
});

function actions(access: DashboardAccess = "owner") {
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
    ["owner", true, true],
    ["ownerless", false, true],
    ["viewer", false, false],
  ] as const)(
    "a caller with %s access is offered Share: %s, Edit: %s",
    (access, share, edit) => {
      actions(access);

      expect(
        screen.queryByRole("button", { name: "Share delivery" }) !== null
      ).toBe(share);
      expect(
        screen.queryByRole("button", { name: "Edit delivery" }) !== null
      ).toBe(edit);
      expect(
        screen.getByRole("button", { name: "More for delivery" })
      ).toBeInTheDocument();
    }
  );

  it("opens the share dialog for the owner", async () => {
    const user = userEvent.setup();
    actions("owner");

    await user.click(screen.getByRole("button", { name: "Share delivery" }));

    expect(await screen.findByRole("dialog", { name: "Share" })).toHaveTextContent(
      OWNER
    );
  });

  it("offers pin, rename, duplicate, filing and delete to the owner", async () => {
    await openMenu();

    await screen.findByRole("menuitem", { name: "Pin" });
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual([
      "Pin",
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
    expect(customClient.renameDefinition).toHaveBeenCalledWith(
      "dashboards",
      ID,
      "shipping"
    );
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
      customClient.renameDefinition,
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
