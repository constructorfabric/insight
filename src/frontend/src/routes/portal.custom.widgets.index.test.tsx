vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/api/custom-client");

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.widgets.index";

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

describe("/portal/custom/widgets", () => {
  it("shows a table widget's columns and links its metric by id, under its name", async () => {
    vi.mocked(customClient.fetchWidgets).mockResolvedValue({
      items: [{ id: "commits_table", name: "commits_table" }],
      total: 1,
    });
    vi.mocked(customClient.fetchWidget).mockResolvedValue({
      name: "commits_table",
      widget: {
        type: "table",
        metric: "commits_per_day",
        columns: ["day", "lines"],
      },
    });
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      name: "Commits per day",
      definition: { dataset: "commits", fields: [] },
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("commits_table")).toBeInTheDocument();
    expect(await screen.findByText("table")).toBeInTheDocument();
    expect(await screen.findByText("day, lines")).toBeInTheDocument();
    expect(
      await screen.findByRole("link", { name: "Commits per day" })
    ).toHaveAttribute("href", "/portal/custom/metrics/commits_per_day");
  });

  it("shows a line widget's axes instead of columns", async () => {
    vi.mocked(customClient.fetchWidgets).mockResolvedValue({
      items: [{ id: "commits_graph", name: "commits_graph" }],
      total: 1,
    });
    vi.mocked(customClient.fetchWidget).mockResolvedValue({
      name: "commits_graph",
      widget: {
        type: "line",
        metric: "commits_per_day",
        x: "day",
        y: "lines",
      },
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("line")).toBeInTheDocument();
    expect(await screen.findByText(/x day/)).toBeInTheDocument();
    expect(screen.queryByText("Columns")).not.toBeInTheDocument();
  });

  it("says so when there are no widgets yet", async () => {
    vi.mocked(customClient.fetchWidgets).mockResolvedValue({ items: [], total: 0 });

    render(<Component />, { wrapper });

    expect(await screen.findByText(/no widgets yet/i)).toBeInTheDocument();
  });

  it("shows a retryable error when the list fails to load", async () => {
    vi.mocked(customClient.fetchWidgets).mockRejectedValue(
      new Error("network down")
    );

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: /retry/i })
    ).toBeInTheDocument();
  });
});
