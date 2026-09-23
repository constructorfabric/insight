vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    putDataset: vi.fn(),
    fetchDataset: vi.fn(),
    fetchTables: vi.fn(),
    fetchTable: vi.fn(),
    fetchMetricNames: vi.fn(),
    fetchWidgetNames: vi.fn(),
    fetchDashboardNames: vi.fn(),
    fetchDatasetNames: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.new.$kind";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

const NO_NAMES = { names: [], total: 0 };

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/new/datasets");
  for (const names of [
    customClient.fetchMetricNames,
    customClient.fetchWidgetNames,
    customClient.fetchDashboardNames,
    customClient.fetchDatasetNames,
  ]) {
    vi.mocked(names).mockResolvedValue(NO_NAMES);
  }
});

describe("/portal/custom/new/$kind", () => {
  // Nothing is read: there is no definition yet, and a form that waited on one
  // would never open.
  it("offers an empty form without asking for a stored definition", () => {
    render(<Component />, { wrapper });

    expect(screen.getByRole("heading")).toHaveTextContent("New dataset");
    expect(screen.getByLabelText("Name")).toHaveValue("");
    expect(customClient.fetchDataset).not.toHaveBeenCalled();
  });

  it("stores what was written under the name it was given", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDataset).mockResolvedValue({
      name: "deployments",
      declaration: { title: "Deployments", fields: [] },
    });

    render(<Component />, { wrapper });

    await user.type(screen.getByLabelText("Name"), "deployments");
    await user.type(screen.getByLabelText("Title"), "Deployments");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDataset).toHaveBeenCalledWith("deployments", {
        title: "Deployments",
      })
    );
    expect(portalRouter.navigations).toContainEqual({
      to: "/portal/custom/datasets",
    });
  });

  it("will not store anything until it has been named", () => {
    render(<Component />, { wrapper });

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });
});
