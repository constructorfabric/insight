import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  queryMetricResults,
  type MetricResultsRequest,
  type MetricResultsResponse,
} from "@/api/metric-results-client";
import type { MetricCollectionConfig } from "@/lib/metrics/collection";
import { useMemberGridData } from "@/queries/member-grid";

vi.mock("@/api/metric-results-client", async (orig) => ({
  ...(await orig<typeof import("@/api/metric-results-client")>()),
  queryMetricResults: vi.fn(),
}));

const mock = vi.mocked(queryMetricResults);

// Every requested view echoed back, with a comparison reading so the trend
// arrows have real data.
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
      views: m.views.map((v) =>
        v.view === "period"
          ? {
              view: "period",
              values: ids.map((id) => ({
                entity_id: id,
                value: 1,
                ...(req.compare_to ? { compare_to: 10 } : {}),
              })),
            }
          : {
              view: "peer",
              values: ids.map((id) => ({
                entity_id: id,
                target_value: 1,
                p25: 0,
                median: 1,
                p75: 2,
                min: 0,
                max: 3,
                n: 8,
              })),
            }
      ),
    })),
  };
}

const COLLECTION: MetricCollectionConfig = {
  metrics: [{ key: "git.commits", views: [{ view: "period" }, { view: "peer" }] }],
};
const RANGE = { from: "2026-04-01", to: "2026-04-30" };

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
}

describe("useMemberGridData", () => {
  beforeEach(() => {
    mock.mockReset();
    mock.mockImplementation(async (req) => respond(req));
  });

  it("serves the grid and its trend arrows from one request", async () => {
    const { result } = renderHook(
      () =>
        useMemberGridData(
          COLLECTION,
          { type: "person", ids: ["a@x.com"] },
          RANGE,
          "month"
        ),
      { wrapper: wrapper() }
    );

    await waitFor(() => {
      expect(result.current.isPending).toBe(false);
      expect(result.current.byKey.has("git.commits")).toBe(true);
      expect(result.current.previousByKey.has("git.commits")).toBe(true);
    });

    // One request, carrying period + peer over the range and the previous
    // period as its comparison window — not a second fetch over a shifted
    // range.
    expect(mock).toHaveBeenCalledTimes(1);
    const req = mock.mock.calls[0]![0];
    expect(req.period.from).toBe(RANGE.from);
    expect(req.metrics[0]?.views.map((v) => v.view)).toEqual([
      "period",
      "peer",
    ]);
    expect(req.compare_to).toEqual({ from: "2026-03-01", to: "2026-03-30" });
    expect(
      result.current.previousByKey.get("git.commits")?.period?.values[0]?.value,
    ).toBe(10);
  });

  it("is pending with no entities and does no fetch", () => {
    mock.mockClear();
    const { result } = renderHook(
      () =>
        useMemberGridData(COLLECTION, { type: "person", ids: [] }, RANGE, "month"),
      { wrapper: wrapper() }
    );
    expect(result.current.byKey.size).toBe(0);
    expect(mock).not.toHaveBeenCalled();
  });
});
