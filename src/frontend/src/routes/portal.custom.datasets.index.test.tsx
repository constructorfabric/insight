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
import { scrollEndOutOfView } from "@/test/intersection-observer";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.datasets.index";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

const COMMITS = {
  name: "commits",
  declaration: {
    title: "Commits",
    fields: [
      { name: "author", path: "author", type: "string" as const },
      {
        name: "day",
        path: "day",
        type: "datetime" as const,
        default_clock: true,
      },
    ],
    row_identity: ["author"],
  },
};

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset();
  scrollEndOutOfView();
});

describe("/portal/custom/datasets", () => {
  it("names every dataset and reads its declaration back", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits"],
      total: 1,
    });
    vi.mocked(customClient.fetchDataset).mockResolvedValue(COMMITS);

    render(<Component />, { wrapper });

    expect(await screen.findByText("commits")).toBeInTheDocument();
    expect(await screen.findByText("Commits")).toBeInTheDocument();
    expect(
      await screen.findByText("author (string), day (datetime)")
    ).toBeInTheDocument();
  });

  // Which field a window selects by, and which records count as one, are the
  // two things a reader cannot infer from the field list.
  it("says which date a window uses and what makes two records one", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits"],
      total: 1,
    });
    vi.mocked(customClient.fetchDataset).mockResolvedValue(COMMITS);

    render(<Component />, { wrapper });

    expect(await screen.findByText("Main date")).toBeInTheDocument();
    expect(await screen.findByText("One record per")).toBeInTheDocument();
  });

  it("says so when a declaration cannot be read", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits"],
      total: 1,
    });
    vi.mocked(customClient.fetchDataset).mockRejectedValue(
      new Error("that dataset is gone")
    );

    render(<Component />, { wrapper });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "that dataset is gone"
    );
  });

  it("says there are none rather than showing an empty list", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: [],
      total: 0,
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText(/No datasets yet/)).toBeInTheDocument();
  });
});
