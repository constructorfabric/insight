vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/api/custom-client");

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { datasetNames } from "@/test/catalogue";
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

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset();
  scrollEndOutOfView();
});

async function openMenu(title: string) {
  await userEvent.click(
    await screen.findByRole("button", { name: `More for ${title}` })
  );
  await screen.findAllByRole("menuitem");
}

describe("/portal/custom/datasets", () => {
  it("names a dataset by its title, with its name and how many fields it declares", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits"],
      items: [{ name: "commits", title: "Commits", fields: 2 }],
      total: 1,
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("Commits")).toBeInTheDocument();
    expect(screen.getByText("commits")).toBeInTheDocument();
    expect(screen.getByText("2 fields")).toBeInTheDocument();
    expect(customClient.fetchDataset).not.toHaveBeenCalled();
  });

  it("names an untitled dataset once, by its name", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["events"],
      items: [{ name: "events", title: null, fields: 1 }],
      total: 1,
    });

    render(<Component />, { wrapper });

    expect(await screen.findAllByText("events")).toHaveLength(1);
    expect(screen.getByText("1 field")).toBeInTheDocument();
  });

  it("opens a dataset by its name, and deletes it with its records once confirmed", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits"],
      items: [{ name: "commits", title: "Commits", fields: 2 }],
      total: 1,
    });
    vi.mocked(customClient.deleteDataset).mockResolvedValue(undefined);

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("link", { name: "Open Commits" })
    ).toHaveAttribute("href", "/portal/custom/datasets/commits");
    await openMenu("Commits");
    await userEvent.click(screen.getByRole("menuitem", { name: "Delete…" }));
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(/records it holds are deleted/)
    ).toBeInTheDocument();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Delete dataset" })
    );

    expect(customClient.deleteDataset).toHaveBeenCalledWith("commits");
  });

  it("says there are none rather than showing an empty list", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue(
      datasetNames([])
    );

    render(<Component />, { wrapper });

    expect(await screen.findByText(/no datasets yet/i)).toBeInTheDocument();
  });
});
