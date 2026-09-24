vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchMetric: vi.fn(),
    fetchWidget: vi.fn(),
    runMetric: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";

import { WidgetDrilldown } from "./widget-drilldown";

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

/** A metric a window can select by, and the date it would use. */
const CLOCKED = {
  definition: {
    dataset: "pull_requests",
    time: { field: "merged_at" },
    fields: [{ field: "id", type: "int", agg: "count", as_name: "merged" }],
  },
  clock: { field: "merged_at", from: "metric" as const },
};

/** The same metric over a dataset that marks no date: nothing windows it. */
const CLOCKLESS = {
  definition: {
    dataset: "pull_requests",
    fields: [{ field: "id", type: "int", agg: "count", as_name: "total" }],
  },
};

beforeEach(() => {
  vi.resetAllMocks();
});

describe("<WidgetDrilldown>", () => {
  it("reads the window the card read, so the two cannot disagree", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);
    vi.mocked(customClient.runMetric).mockResolvedValue({
      columns: ["merged"],
      rows: [[9]],
    });

    render(
      <WidgetDrilldown
        widget={{
          type: "stat",
          metric: "merged",
          value: "merged",
          label: "Merged",
        }}
        name="merged_stat"
        label="Merged in this window"
        open
        onOpenChange={vi.fn()}
        options={{ range: "P30D", bucket: false }}
      />,
      { wrapper },
    );

    await vi.waitFor(() => {
      expect(customClient.runMetric).toHaveBeenCalledWith("merged", {
        range: "P30D",
        bucket: false,
      });
    });
  });

  it("does not window a metric that carries no clock of its own", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.runMetric).mockResolvedValue({
      columns: ["total"],
      rows: [[144]],
    });

    render(
      <WidgetDrilldown
        widget={{
          type: "stat",
          metric: "total",
          value: "total",
          label: "Total",
        }}
        name="all_stat"
        label="Pull requests ever opened"
        open
        onOpenChange={vi.fn()}
        options={{ range: "P30D", bucket: false }}
      />,
      { wrapper },
    );

    await vi.waitFor(() => {
      expect(customClient.runMetric).toHaveBeenCalledWith("total", undefined);
    });
  });

  // The dialog is sized for a result of a few columns. A wider one is read by
  // filling the window rather than by scrolling a small frame.
  it("fills the window when asked, and gives the room back", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.runMetric).mockResolvedValue({
      columns: ["total"],
      rows: [[144]],
    });

    render(
      <WidgetDrilldown
        widget={{ type: "table", metric: "total", columns: ["total"] }}
        name="all_table"
        label="Pull requests ever opened"
        open
        onOpenChange={vi.fn()}
      />,
      { wrapper },
    );

    const dialog = await screen.findByRole("dialog");
    const width = () => dialog.className;

    expect(width()).toContain("80rem");

    await user.click(screen.getByRole("button", { name: "Fill the window" }));

    expect(width()).toContain("98vw");
    expect(width()).not.toContain("80rem");

    await user.click(
      screen.getByRole("button", { name: "Shrink the dialog back" })
    );

    expect(width()).toContain("80rem");
  });
});
