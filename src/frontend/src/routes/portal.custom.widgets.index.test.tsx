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
import { widgetListed } from "@/test/catalogue";
import {
  scrollEndIntoView,
  scrollEndOutOfView,
} from "@/test/intersection-observer";
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
  scrollEndOutOfView();
});

async function openMenu(title: string) {
  await userEvent.click(
    await screen.findByRole("button", { name: `More for ${title}` })
  );
  await screen.findAllByRole("menuitem");
}

describe("/portal/custom/widgets", () => {
  it.each([
    [{ type: "table" as const, metric: "m-1", columns: ["day"] }, "Table"],
    [
      { type: "line" as const, metric: "m-1", x: "bucket", y: "commits" },
      "Line chart",
    ],
    [
      { type: "bar" as const, metric: "m-1", x: "repo", y: "lines" },
      "Bar chart",
    ],
    [
      { type: "area" as const, metric: "m-1", x: "bucket", y: "lines" },
      "Area chart",
    ],
    [{ type: "stat" as const, metric: "m-1", value: "total" }, "Figure"],
    [
      { type: "pie" as const, metric: "m-1", label: "repo", value: "lines" },
      "Pie chart",
    ],
  ])(
    "says what a %o widget draws and over which metric",
    async (body, label) => {
      vi.mocked(customClient.fetchWidgets).mockResolvedValue({
        items: [widgetListed("w-1", "commits_widget", body, "Commits per day")],
        total: 1,
      });

      render(<Component />, { wrapper });

      expect(
        await screen.findByText(label),
        `label: ${label}`
      ).toBeInTheDocument();
      expect(screen.getByText("Commits per day")).toBeInTheDocument();
    }
  );

  it("names a widget by its title, or its name without one", async () => {
    vi.mocked(customClient.fetchWidgets).mockResolvedValue({
      items: [
        widgetListed("w-1", "titled_widget", {
          type: "table",
          metric: "m-1",
          columns: [],
          title: "Commits by day",
        }),
        widgetListed("w-2", "untitled_widget"),
      ],
      total: 2,
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("Commits by day")).toBeInTheDocument();
    expect(screen.getByText("untitled_widget")).toBeInTheDocument();
    expect(screen.queryByText("titled_widget")).toBeNull();
  });

  it("says when the metric a widget draws no longer exists", async () => {
    vi.mocked(customClient.fetchWidgets).mockResolvedValue({
      items: [widgetListed("w-1", "orphan", undefined, null)],
      total: 1,
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("Missing metric")).toBeInTheDocument();
  });

  it("draws every card from the list, reading neither widgets nor metrics", async () => {
    vi.mocked(customClient.fetchWidgets).mockResolvedValue({
      items: [widgetListed("w-1"), widgetListed("w-2")],
      total: 2,
    });

    render(<Component />, { wrapper });

    expect(await screen.findByText("w-2")).toBeInTheDocument();
    expect(customClient.fetchWidget).not.toHaveBeenCalled();
    expect(customClient.fetchMetric).not.toHaveBeenCalled();
  });

  it("opens a widget by its id, and offers to open, edit or delete it", async () => {
    vi.mocked(customClient.fetchWidgets).mockResolvedValue({
      items: [widgetListed("w-1", "Commits")],
      total: 1,
    });

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("link", { name: "Open Commits" })
    ).toHaveAttribute("href", "/portal/custom/widgets/w-1");
    await openMenu("Commits");
    expect(
      screen.getAllByRole("menuitem").map((item) => item.textContent)
    ).toEqual(["Open", "Edit", "Delete…"]);
  });

  it("says so when there are no widgets yet", async () => {
    vi.mocked(customClient.fetchWidgets).mockResolvedValue({
      items: [],
      total: 0,
    });

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

  it("reads the next page at the end of the list", async () => {
    vi.mocked(customClient.fetchWidgets).mockImplementation(
      async ({ offset = 0 } = {}) =>
        offset === 0
          ? { items: [widgetListed("first_page")], total: 2 }
          : { items: [widgetListed("second_page")], total: 2 }
    );

    render(<Component />, { wrapper });
    await screen.findByText("first_page");
    scrollEndIntoView();

    expect(await screen.findByText("second_page")).toBeInTheDocument();
  });
});
