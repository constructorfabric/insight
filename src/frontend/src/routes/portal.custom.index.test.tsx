vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/api/custom-client");

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
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

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset();
});

describe("/portal/custom", () => {
  it("lists every dashboard as a link", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [
      "engineering",
      "delivery",
    ], total: 2 });
    vi.mocked(customClient.fetchDashboard).mockRejectedValue(
      new Error("no title today")
    );

    render(<Component />, { wrapper });

    // The identifier stands in until a title arrives, so the link is still
    // reachable when a definition cannot be read.
    expect(
      await screen.findByRole("link", { name: "engineering" })
    ).toHaveAttribute("href", "/portal/custom/engineering");
    expect(
      await screen.findByRole("link", { name: "delivery" })
    ).toBeInTheDocument();
  });

  it("titles a card by the dashboard, keeping the identifier beneath it", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [
      "lines_of_code_dashboard",
    ], total: 1 });
    vi.mocked(customClient.fetchDashboard).mockResolvedValue({
      title: "Lines of Code",
      widgets: [],
    });

    render(<Component />, { wrapper });

    // The reader picks a dashboard by its name, not by its slug.
    expect(await screen.findByText("Lines of Code")).toBeInTheDocument();
    expect(
      await screen.findByText("lines_of_code_dashboard")
    ).toBeInTheDocument();
  });

  it("says so when there are no dashboards yet", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [], total: 0 });

    render(<Component />, { wrapper });

    expect(await screen.findByText(/no dashboards yet/i)).toBeInTheDocument();
  });

  it("shows a loading state before the list resolves", () => {
    vi.mocked(customClient.fetchDashboardNames).mockReturnValue(
      new Promise(() => {})
    );

    render(<Component />, { wrapper });

    expect(
      screen.getByRole("status", { name: /loading/i })
    ).toBeInTheDocument();
  });

  it("shows a retryable error state when the list fails to load", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockRejectedValue(
      new Error("network down")
    );

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: /retry/i })
    ).toBeInTheDocument();
  });
});

describe("/portal/custom in a folder", () => {
  const PLATFORM = { id: "f1", name: "Platform", dashboards: 1 };

  beforeEach(() => {
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: [PLATFORM],
      unfiled: 2,
    });
    vi.mocked(customClient.fetchDashboard).mockRejectedValue(new Error("untitled"));
  });

  const askedFor = () =>
    vi.mocked(customClient.fetchDashboardNames).mock.calls.map(([page]) => page?.folder);

  it("lists one folder's dashboards and names it in the heading", async () => {
    portalRouter.set({ folder: "f1" });
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["delivery"], total: 1 });

    render(<Component />, { wrapper });

    expect(await screen.findByRole("heading", { level: 1, name: "Platform" })).toBeInTheDocument();
    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual(["f1"]);
  });

  it("lists the unfiled under a heading of their own", async () => {
    portalRouter.set({ folder: "unfiled" });
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["hiring"], total: 1 });

    render(<Component />, { wrapper });

    expect(await screen.findByRole("heading", { level: 1, name: "Unfiled" })).toBeInTheDocument();
    await screen.findByRole("link", { name: "hiring" });
    expect(askedFor()).toEqual(["unfiled"]);
  });

  it("shows every dashboard for a folder that is not there, and says so", async () => {
    portalRouter.set({ folder: "gone" });
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["delivery"], total: 1 });

    render(<Component />, { wrapper });

    expect(await screen.findByText(/that folder no longer exists/i)).toBeInTheDocument();
    await screen.findByRole("link", { name: "delivery" });
    expect(askedFor()).toEqual([undefined]);
    expect(screen.getByRole("heading", { level: 1, name: "Custom" })).toBeInTheDocument();
  });

  it("checks the folder a dashboard is in", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["delivery"], total: 1 });
    vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue({ id: "f1", name: "Platform" });
    const user = userEvent.setup();

    render(<Component />, { wrapper });
    await user.click(await screen.findByRole("button", { name: "More for delivery" }));

    await waitFor(() =>
      expect(screen.getByRole("menuitemradio", { name: /Platform/ })).toHaveAttribute("aria-checked", "true"),
    );
    expect(screen.getByRole("menuitemradio", { name: /Unfiled/ })).toHaveAttribute("aria-checked", "false");
  });

  it("moves a dashboard out of the folder on screen, and the card leaves", async () => {
    portalRouter.set({ folder: "f1" });
    vi.mocked(customClient.fetchDashboardNames)
      .mockResolvedValueOnce({ names: ["delivery"], total: 1 })
      .mockResolvedValue({ names: [], total: 0 });
    vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue({ id: "f1", name: "Platform" });
    vi.mocked(customClient.moveDashboard).mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(<Component />, { wrapper });
    await user.click(await screen.findByRole("button", { name: "More for delivery" }));
    await user.click(await screen.findByRole("menuitemradio", { name: /Unfiled/ }));

    expect(customClient.moveDashboard).toHaveBeenCalledWith("delivery", null);
    await waitFor(() => expect(screen.queryByRole("link", { name: "delivery" })).toBeNull());
  });

  it("makes a folder and then moves the dashboard into it", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: ["delivery"], total: 1 });
    vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue(null);
    vi.mocked(customClient.createFolder).mockResolvedValue({ id: "f9", name: "Hiring" });
    vi.mocked(customClient.moveDashboard).mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(<Component />, { wrapper });
    await user.click(await screen.findByRole("button", { name: "More for delivery" }));
    await user.click(await screen.findByRole("menuitem", { name: /New folder/ }));
    await user.type(await screen.findByRole("textbox", { name: "Folder name" }), "Hiring");
    await user.click(screen.getByRole("button", { name: "Create and move" }));

    await waitFor(() => expect(customClient.moveDashboard).toHaveBeenCalledWith("delivery", "f9"));
    expect(customClient.createFolder).toHaveBeenCalledWith("Hiring");
    expect(vi.mocked(customClient.createFolder).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(customClient.moveDashboard).mock.invocationCallOrder[0],
    );
  });
});
