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
    renameDefinition: vi.fn(),
    duplicateDashboard: vi.fn(),
    deleteDefinition: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { DashboardPageActions } from "./dashboard-page-actions";

const { CustomApiError } = customClient;

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/dash-1");
  vi.mocked(customClient.fetchPins).mockResolvedValue([]);
  vi.mocked(customClient.pinDashboard).mockResolvedValue(undefined);
  vi.mocked(customClient.unpinDashboard).mockResolvedValue(undefined);
  vi.mocked(customClient.renameDefinition).mockResolvedValue(undefined);
  vi.mocked(customClient.duplicateDashboard).mockResolvedValue("dash-2");
  vi.mocked(customClient.deleteDefinition).mockResolvedValue(undefined);
});

function actions() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <DashboardPageActions id="dash-1" name="delivery" title="Delivery" />
    </QueryClientProvider>
  );
}

async function openMenu() {
  const user = userEvent.setup();
  actions();

  await user.click(screen.getByRole("button", { name: "More for delivery" }));
  await screen.findByRole("menu");

  return user;
}

async function choose(user: ReturnType<typeof userEvent.setup>, item: string) {
  await user.click(await screen.findByRole("menuitem", { name: item }));
  return screen.findByRole("dialog");
}

describe("<DashboardPageActions>", () => {
  it("links Edit to the dashboard's editor by its id", () => {
    actions();

    expect(screen.getByRole("button", { name: "Edit delivery" })).toHaveAttribute(
      "href",
      "/portal/custom/edit/dashboards/dash-1"
    );
  });

  it("offers pin, rename, duplicate and delete, and no filing", async () => {
    await openMenu();

    await screen.findByRole("menuitem", { name: "Pin" });
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual(["Pin", "Rename…", "Duplicate…", "Delete…"]);
    expect(screen.queryByRole("menuitemradio")).toBeNull();
  });

  it("pins the dashboard on screen", async () => {
    const user = await openMenu();

    const item = await screen.findByRole("menuitem", { name: "Pin" });
    await waitFor(() => expect(item).not.toHaveAttribute("aria-disabled"));
    await user.click(item);

    expect(customClient.pinDashboard).toHaveBeenCalledWith("dash-1");
  });

  it("unpins the dashboard on screen", async () => {
    vi.mocked(customClient.fetchPins).mockResolvedValue([
      { id: "dash-1", name: "delivery" },
    ]);
    const user = await openMenu();

    await user.click(await screen.findByRole("menuitem", { name: "Unpin" }));

    expect(customClient.unpinDashboard).toHaveBeenCalledWith("dash-1");
  });

  it("stays on the dashboard's page after a rename, which keeps its id", async () => {
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
      "dash-1",
      "shipping"
    );
    expect(portalRouter.navigations).toHaveLength(0);
  });

  it("lands on the copy after a duplicate", async () => {
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

  it("stays on the page and says why when a rename is refused", async () => {
    vi.mocked(customClient.renameDefinition).mockRejectedValue(
      new CustomApiError(400, { detail: "name must be 1 to 128 characters" })
    );
    const user = await openMenu();

    const dialog = await choose(user, "Rename…");
    const field = within(dialog).getByRole("textbox", {
      name: "Dashboard name",
    });
    await user.clear(field);
    await user.type(field, "shipping{Enter}");

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "name must be 1 to 128 characters"
    );
    expect(portalRouter.navigations).toHaveLength(0);
  });

  it.each([
    [
      "Duplicate…",
      "Duplicate",
      customClient.duplicateDashboard,
      "the dashboard is gone",
    ],
    [
      "Delete…",
      "Delete dashboard",
      customClient.deleteDefinition,
      "no dashboard named delivery",
    ],
  ])(
    "stays on the page and says why when %s is refused",
    async (item, confirm, call, said) => {
      vi.mocked(call).mockRejectedValue(
        new CustomApiError(409, { detail: said })
      );
      const user = await openMenu();

      const dialog = await choose(user, item);
      await user.click(within(dialog).getByRole("button", { name: confirm }));

      expect(await within(dialog).findByRole("alert")).toHaveTextContent(said);
      expect(portalRouter.navigations).toHaveLength(0);
    }
  );
});
