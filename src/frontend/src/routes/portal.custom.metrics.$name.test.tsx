vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return { ...actual, fetchMetric: vi.fn(), runMetric: vi.fn() };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.metrics.$name";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

const CLOCKED = {
  definition: {
    dataset: "commits",
    fields: [
      { field: "author", type: "string", as_name: "author" },
      { field: "lines", type: "int", agg: "sum", as_name: "lines" },
    ],
    group_by: ["author"],
  },
  clock: { field: "day", from: "dataset" as const },
};

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/metrics/lines_by_author");
  vi.mocked(customClient.runMetric).mockResolvedValue({
    columns: ["author", "lines"],
    rows: [
      ["ada", 120],
      ["grace", 80],
    ],
  });
});

describe("/portal/custom/metrics/$name", () => {
  it("runs the metric and shows its rows, all time first", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);

    render(<Component />, { wrapper });

    expect(await screen.findByText("ada")).toBeInTheDocument();
    expect(screen.getByText("2 rows")).toBeInTheDocument();
    expect(customClient.runMetric).toHaveBeenCalledWith(
      "lines_by_author",
      undefined
    );
    expect(
      screen.getByText("Windowed by day, the dataset's main date.")
    ).toBeInTheDocument();
  });

  it("runs again over the window the reader picks", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);

    render(<Component />, { wrapper });
    await screen.findByText("ada");

    await user.click(screen.getByRole("button", { name: "Last 30 days" }));

    await waitFor(() =>
      expect(customClient.runMetric).toHaveBeenCalledWith("lines_by_author", {
        range: "P30D",
      })
    );
    expect(
      screen.getByRole("button", { name: "Last 30 days" })
    ).toHaveAttribute("aria-pressed", "true");
  });

  // A window needs a date to select by; a metric with none has nothing to offer.
  it("offers no windows for a metric nothing dates", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      definition: CLOCKED.definition,
    });

    render(<Component />, { wrapper });
    await screen.findByText("ada");

    expect(
      screen.queryByRole("button", { name: "Last 30 days" })
    ).not.toBeInTheDocument();
    expect(
      screen.getByText("Nothing dates this metric: every run reads all time.")
    ).toBeInTheDocument();
  });

  it("shows the service's refusal when the run fails", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);
    vi.mocked(customClient.runMetric).mockRejectedValue(
      new CustomApiError(400, { detail: "the dataset is being removed" })
    );

    render(<Component />, { wrapper });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "the dataset is being removed"
    );
  });

  it("says which metric is missing rather than running anything", async () => {
    vi.mocked(customClient.fetchMetric).mockRejectedValue(
      new CustomApiError(404, null)
    );

    render(<Component />, { wrapper });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "That metric is not there."
    );
    expect(customClient.runMetric).not.toHaveBeenCalled();
  });
});
