import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  queryMetricResults,
  type MetricResultsRequest,
  type MetricResultsResponse,
} from "@/api/metric-results-client";
import { useMetricsViewData } from "@/queries/metrics-view";

vi.mock("@/api/metric-results-client", async (orig) => ({
  ...(await orig<typeof import("@/api/metric-results-client")>()),
  queryMetricResults: vi.fn(),
}));

const mock = vi.mocked(queryMetricResults);

function respond(req: MetricResultsRequest): MetricResultsResponse {
  if (req.entity.type !== "person") throw new Error("person request expected");
  const ids = req.entity.ids;

  return {
    metrics: req.metrics.map((m) => ({
      metric_key: m.metric_key,
      label: m.metric_key,
      unit: null,
      format: "integer",
      direction: "higher_is_better",
      computation: "sum",
      views: [
        {
          view: "period",
          values: ids.map((id) => ({ entity_id: id, value: 1 })),
        },
      ],
    })),
  };
}

const RANGE = { from: "2026-04-01", to: "2026-04-30" };
const KEYS = Array.from({ length: 120 }, (_, i) => `git.m${i}`);

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
}

describe("useMetricsViewData", () => {
  beforeEach(() => {
    mock.mockReset();
    mock.mockImplementation(async (req) => respond(req));
  });

  it("asks for at most 50 metrics per request and serves all of them", async () => {
    const { result } = renderHook(
      () => useMetricsViewData(KEYS, ["a@x.com", "b@x.com"], RANGE),
      { wrapper: wrapper() },
    );

    await waitFor(() => {
      expect(result.current.isPending).toBe(false);
      expect(result.current.byKey.size).toBe(120);
    });

    const sizes = mock.mock.calls.map(([req]) => req.metrics.length);
    expect(sizes.sort((a, b) => b - a)).toEqual([50, 50, 20]);
  });

  it("asks for the period value only, with no comparison window", async () => {
    const { result } = renderHook(
      () => useMetricsViewData(["git.commits"], ["a@x.com"], RANGE),
      { wrapper: wrapper() },
    );

    await waitFor(() => expect(result.current.byKey.has("git.commits")).toBe(true));

    const req = mock.mock.calls[0]![0];
    expect(req.metrics[0]?.views.map((v) => v.view)).toEqual(["period"]);
    expect(req.compare_to).toBeUndefined();
  });

  it("does not fetch when the scope has no people", () => {
    const { result } = renderHook(() => useMetricsViewData(KEYS, [], RANGE), {
      wrapper: wrapper(),
    });

    expect(result.current.byKey.size).toBe(0);
    expect(mock).not.toHaveBeenCalled();
  });
});
