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
    createDefinition: vi.fn(),
    fetchDataset: vi.fn(),
    fetchMetrics: vi.fn(),
    fetchWidgets: vi.fn(),
    fetchDashboards: vi.fn(),
    fetchDatasetNames: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { datasetNames } from "@/test/catalogue";
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

const NO_ITEMS = { items: [], total: 0 };

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/new/datasets");
  for (const page of [
    customClient.fetchMetrics,
    customClient.fetchWidgets,
    customClient.fetchDashboards,
  ]) {
    vi.mocked(page).mockResolvedValue(NO_ITEMS);
  }
  vi.mocked(customClient.fetchDatasetNames).mockResolvedValue(datasetNames([]));
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

  it("stores a dataset under the name it was given, then opens it", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDataset).mockResolvedValue({
      name: "deployments",
      declaration: { title: "Deployments", fields: [] },
    });

    render(<Component />, { wrapper });

    await user.type(screen.getByLabelText("Name"), "deployments");
    await user.click(screen.getByLabelText(/as text/i));
    await user.clear(screen.getByLabelText(/as text/i));
    await user.paste('{"title": "Deployments"}');
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDataset).toHaveBeenCalledWith("deployments", {
        title: "Deployments",
      })
    );
    await waitFor(() =>
      expect(portalRouter.navigations).toContainEqual({
        to: "/portal/custom/datasets/$name",
        params: { name: "deployments" },
      })
    );
  });

  it.each([
    ["metrics", "/portal/custom/metrics/$name"],
    ["widgets", "/portal/custom/widgets/$name"],
    ["dashboards", "/portal/custom/$name"],
  ])("creates a new %s and opens it by the id it was given", async (kind, page) => {
    const user = userEvent.setup();
    portalRouter.reset(`/portal/custom/new/${kind}`);
    vi.mocked(customClient.createDefinition).mockResolvedValue(
      "0192f0aa-0000-7000-8000-000000000004"
    );

    render(<Component />, { wrapper });

    const text = { metrics: "Commits", widgets: "Commits", dashboards: "Board" };
    await user.type(screen.getByLabelText("Name"), text[kind as keyof typeof text]);
    const document = screen.getByLabelText(/as text/i);
    await user.clear(document);
    await user.type(document, '{{"title": "Board"}');
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.createDefinition).toHaveBeenCalledWith(
        kind,
        text[kind as keyof typeof text],
        { title: "Board" }
      )
    );
    await waitFor(() =>
      expect(portalRouter.navigations).toContainEqual({
        to: page,
        params: { name: "0192f0aa-0000-7000-8000-000000000004" },
      })
    );
  });

  it("will not store anything until it has been named", () => {
    render(<Component />, { wrapper });

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });
});
