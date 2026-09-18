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
    fetchWidget: vi.fn(),
    fetchMetric: vi.fn(),
    runMetric: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.widgets.$name";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

const TABLE = {
  type: "table" as const,
  metric: "lines_by_author",
  columns: ["author", "lines"],
  title: "Lines by author",
};

const CLOCKED = {
  definition: {
    dataset: "commits",
    fields: [{ field: "lines", type: "int", agg: "sum", as_name: "lines" }],
  },
  clock: { field: "day", from: "dataset" as const },
};

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/widgets/lines_table");
  vi.mocked(customClient.fetchWidget).mockResolvedValue(TABLE);
  vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);
  vi.mocked(customClient.runMetric).mockResolvedValue({
    columns: ["author", "lines"],
    rows: [["ada", 120]],
  });
});

describe("/portal/custom/widgets/$name", () => {
  it("draws the widget as a board would, all time first", async () => {
    render(<Component />, { wrapper });

    expect(await screen.findByText("ada")).toBeInTheDocument();
    expect(screen.getByText("Lines by author")).toBeInTheDocument();
    expect(customClient.runMetric).toHaveBeenCalledWith(
      "lines_by_author",
      undefined
    );
  });

  // A chart over time is drawn from buckets; a table is not.
  it("runs over the window picked, sliced as the chart draws it", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchWidget).mockResolvedValue({
      type: "line",
      metric: "lines_by_author",
      x: "bucket",
      y: "lines",
    });

    render(<Component />, { wrapper });
    await screen.findByRole("button", { name: "Last 30 days" });

    await user.click(screen.getByRole("button", { name: "Last 30 days" }));

    await waitFor(() =>
      expect(customClient.runMetric).toHaveBeenCalledWith("lines_by_author", {
        range: "P30D",
        bucket: true,
      })
    );
  });

  it("says that a window means all time when nothing dates the metric", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      definition: CLOCKED.definition,
    });

    render(<Component />, { wrapper });
    await screen.findByText("ada");

    await user.click(screen.getByRole("button", { name: "Last 30 days" }));

    expect(
      await screen.findByText(/every window shows all time/)
    ).toBeInTheDocument();
    expect(customClient.runMetric).toHaveBeenCalledTimes(1);
  });

  it("shows the service's refusal when the metric cannot run", async () => {
    vi.mocked(customClient.runMetric).mockRejectedValue(
      new CustomApiError(400, { detail: "the dataset is being removed" })
    );

    render(<Component />, { wrapper });

    expect(
      await screen.findByText("the dataset is being removed")
    ).toBeInTheDocument();
  });

  it("says which widget is missing", async () => {
    vi.mocked(customClient.fetchWidget).mockRejectedValue(
      new CustomApiError(404, null)
    );

    render(<Component />, { wrapper });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "That widget is not there."
    );
  });
});
