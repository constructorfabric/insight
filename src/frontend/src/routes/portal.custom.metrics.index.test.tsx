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
import { metricListed } from "@/test/catalogue";
import {
  scrollEndIntoView,
  scrollEndOutOfView,
} from "@/test/intersection-observer";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.metrics.index";

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

describe("/portal/custom/metrics", () => {
  it.each([
    [
      "a dataset",
      {
        dataset: "commits",
        fields: [{ field: "lines", type: "int", as_name: "lines" }],
      },
      ["commits", "1 column"],
    ],
    [
      "a table",
      {
        table: "silver.example",
        fields: [
          { column: "repo", type: "string", as_name: "repo" },
          { agg: "count", type: "int", as_name: "total" },
        ],
      },
      ["silver.example", "2 columns"],
    ],
  ])(
    "says what a metric over %s reads and how many columns it answers",
    async (_, body, facts) => {
      vi.mocked(customClient.fetchMetrics).mockResolvedValue({
        items: [metricListed("m-1", "Lines per day", body)],
        total: 1,
      });

      render(<Component />, { wrapper });

      expect(await screen.findByText("Lines per day")).toBeInTheDocument();
      for (const fact of facts) {
        expect(screen.getByText(fact), `fact: ${fact}`).toBeInTheDocument();
      }
    }
  );

  it("draws every card from the list without reading each metric", async () => {
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: [
        metricListed("m-1", "Lines per day"),
        metricListed("m-2", "Commits"),
      ],
      total: 2,
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("Commits")).toBeInTheDocument();
    expect(customClient.fetchMetric).not.toHaveBeenCalled();
  });

  it("opens a metric by its id, and offers to open, edit or delete it", async () => {
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: [metricListed("m-1", "Lines per day")],
      total: 1,
    });

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("link", { name: "Open Lines per day" })
    ).toHaveAttribute("href", "/portal/custom/metrics/m-1");
    await openMenu("Lines per day");
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual(["Open", "Edit", "Delete…"]);
  });

  it("deletes a metric by its id once confirmed, and shows the service's refusal", async () => {
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: [metricListed("m-1", "Lines per day")],
      total: 1,
    });
    vi.mocked(customClient.deleteDefinition).mockRejectedValue(
      new Error("A widget still draws it.")
    );

    render(<Component />, { wrapper });
    await openMenu("Lines per day");
    await userEvent.click(screen.getByRole("menuitem", { name: "Delete…" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Delete metric" })
    );

    expect(customClient.deleteDefinition).toHaveBeenCalledWith(
      "metrics",
      "m-1"
    );
    expect(
      await within(dialog).findByText(/could not be deleted|still draws/)
    ).toBeInTheDocument();
  });

  it("says so when there are no metrics yet", async () => {
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: [],
      total: 0,
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText(/no metrics yet/i)).toBeInTheDocument();
  });

  it("shows a retryable error when the list fails to load", async () => {
    vi.mocked(customClient.fetchMetrics).mockRejectedValue(
      new Error("network down")
    );

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: /retry/i })
    ).toBeInTheDocument();
  });

  it("says how many metrics there are, and reads the next page at the end", async () => {
    vi.mocked(customClient.fetchMetrics).mockImplementation(
      async ({ offset = 0 } = {}) =>
        offset === 0
          ? { items: [metricListed("first_page")], total: 2 }
          : { items: [metricListed("second_page")], total: 2 }
    );

    render(<Component />, { wrapper });

    expect(await screen.findByText("2 metrics")).toBeInTheDocument();
    expect(await screen.findByText("first_page")).toBeInTheDocument();
    expect(screen.queryByText("second_page")).not.toBeInTheDocument();

    scrollEndIntoView();

    expect(await screen.findByText("second_page")).toBeInTheDocument();
  });

  it("counts what a search matched rather than the whole catalogue", async () => {
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: [metricListed("only_one")],
      total: 1,
    });

    render(<Component />, { wrapper });
    await screen.findByText("1 metric");

    await userEvent.type(
      screen.getByRole("searchbox", { name: /search metrics/i }),
      "only"
    );

    expect(await screen.findByText("1 matching")).toBeInTheDocument();
    expect(customClient.fetchMetrics).toHaveBeenCalledWith({
      search: "only",
      limit: 50,
      offset: 0,
    });
  });

  it("starts a new metric from a primary button, without a blurb", async () => {
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: [],
      total: 0,
    });

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("heading", { level: 1, name: "Metrics" })
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New metric" })).toHaveAttribute(
      "href",
      "/portal/custom/new/metrics"
    );
    expect(screen.queryByText(/every stored query/i)).toBeNull();
  });
});
