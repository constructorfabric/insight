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
import { render } from "@testing-library/react";
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

const CLOCKED = {
  table: "demo_pull_requests",
  time: { json: "merged_at" },
  fields: [{ json: "id", type: "int", agg: "count", as_name: "merged" }],
};

const CLOCKLESS = {
  table: "demo_pull_requests",
  fields: [{ json: "id", type: "int", agg: "count", as_name: "total" }],
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

});
