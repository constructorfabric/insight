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
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import {
  scrollEndIntoView,
  scrollEndOutOfView,
} from "@/test/intersection-observer";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.metrics";

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

describe("/portal/custom/metrics", () => {
  it("names every metric and reads its query back", async () => {
    vi.mocked(customClient.fetchMetricNames).mockResolvedValue({ names: [
      "lines_per_day",
    ], total: 1 });
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      table: "events",
      fields: [
        { json: "day", type: "string", as_name: "day" },
        { json: "lines", type: "int", agg: "sum", as_name: "total_lines" },
      ],
      group_by: ["day"],
      filters: [{ json: "event", type: "string", op: "eq", value: "commit" }],
      order_by: { field: "total_lines", direction: "desc" },
      limit: 100,
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("lines_per_day")).toBeInTheDocument();
    expect(await screen.findByText("events")).toBeInTheDocument();
    // The aggregate and its alias, so a reader can tell which column is which.
    expect(
      await screen.findByText("day as day, sum(lines) as total_lines")
    ).toBeInTheDocument();
    expect(await screen.findByText("event eq commit")).toBeInTheDocument();
    // Ordering decides which row answers "the most", so the catalogue says it.
    expect(await screen.findByText("total_lines desc")).toBeInTheDocument();
    expect(await screen.findByText("100")).toBeInTheDocument();
  });

  it("reads back a query over a real table, by column and by person", async () => {
    // Metrics grew columns, a database and a resolved person after this card
    // was written, and it printed `undefined` for every one of them.
    vi.mocked(customClient.fetchMetricNames).mockResolvedValue({ names: [
      "pr_merged_by_author",
    ], total: 1 });
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      database: "silver",
      table: "class_git_pull_requests",
      fields: [
        {
          column: "author_email",
          type: "string",
          as_name: "author_name",
          person: "email",
        },
        { column: "pr_id", type: "int", agg: "count", as_name: "merged" },
      ],
      group_by: ["author_name"],
      filters: [{ column: "state", type: "string", op: "eq", value: "MERGED" }],
    });

    render(<Component />, { wrapper });

    expect(
      await screen.findByText("silver.class_git_pull_requests")
    ).toBeInTheDocument();
    expect(
      await screen.findByText(
        "author_email by name as author_name, count(pr_id) as merged"
      )
    ).toBeInTheDocument();
    expect(await screen.findByText("state eq MERGED")).toBeInTheDocument();
  });

  it("counts rows without naming a column", async () => {
    vi.mocked(customClient.fetchMetricNames).mockResolvedValue({ names: ["how_many"], total: 1 });
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      table: "silver.class_git_commits",
      fields: [{ type: "int", agg: "count", as_name: "commits" }],
      group_by: [],
      filters: [],
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("count() as commits")).toBeInTheDocument();
  });

  it("leaves out the rows a metric does not use", async () => {
    vi.mocked(customClient.fetchMetricNames).mockResolvedValue({ names: ["everything"], total: 1 });
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      table: "events",
      fields: [{ json: "lines", type: "int", as_name: "lines" }],
      group_by: [],
      filters: [],
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("everything")).toBeInTheDocument();
    expect(screen.queryByText("Grouped by")).not.toBeInTheDocument();
    expect(screen.queryByText("Filtered")).not.toBeInTheDocument();
    expect(screen.queryByText("Ordered by")).not.toBeInTheDocument();
    expect(screen.queryByText("Limit")).not.toBeInTheDocument();
  });

  it("says so when there are no metrics yet", async () => {
    vi.mocked(customClient.fetchMetricNames).mockResolvedValue({ names: [], total: 0 });

    render(<Component />, { wrapper });

    expect(await screen.findByText(/no metrics yet/i)).toBeInTheDocument();
  });

  it("shows a retryable error when the list fails to load", async () => {
    vi.mocked(customClient.fetchMetricNames).mockRejectedValue(
      new Error("network down")
    );

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: /retry/i })
    ).toBeInTheDocument();
  });

  it("keeps the rest of the page when one metric fails to load", async () => {
    vi.mocked(customClient.fetchMetricNames).mockResolvedValue({ names: [
      "broken",
      "fine",
    ], total: 2 });
    vi.mocked(customClient.fetchMetric).mockImplementation(async (name) => {
      if (name === "broken") throw new Error("that metric is gone");
      return {
        table: "events",
        fields: [{ json: "lines", type: "int", as_name: "lines" }],
      };
    });

    render(<Component />, { wrapper });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "that metric is gone"
    );
    expect(await screen.findByText("fine")).toBeInTheDocument();
  });
  it("says how many metrics there are, and reads the next page at the end", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      table: "events",
      fields: [{ json: "lines", type: "int", as_name: "lines" }],
    });
    vi.mocked(customClient.fetchMetricNames).mockImplementation(
      async ({ offset = 0 } = {}) =>
        offset === 0
          ? { names: ["first_page"], total: 2 }
          : { names: ["second_page"], total: 2 }
    );

    render(<Component />, { wrapper });

    expect(await screen.findByText("2 metrics")).toBeInTheDocument();
    expect(await screen.findByText("first_page")).toBeInTheDocument();
    expect(screen.queryByText("second_page")).not.toBeInTheDocument();

    scrollEndIntoView();

    expect(await screen.findByText("second_page")).toBeInTheDocument();
  });

  it("counts what a search matched rather than the whole catalogue", async () => {
    vi.mocked(customClient.fetchMetricNames).mockResolvedValue({
      names: ["only_one"],
      total: 1,
    });
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      table: "events",
      fields: [{ json: "lines", type: "int", as_name: "lines" }],
    });

    render(<Component />, { wrapper });
    await screen.findByText("1 metrics");

    await userEvent.type(
      screen.getByRole("searchbox", { name: /search metrics/i }),
      "only"
    );

    expect(await screen.findByText("1 matching")).toBeInTheDocument();
    expect(customClient.fetchMetricNames).toHaveBeenCalledWith({
      search: "only",
      limit: 50,
      offset: 0,
    });
  });
});
