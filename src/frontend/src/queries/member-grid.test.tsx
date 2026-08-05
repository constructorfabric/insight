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

// Every requested view echoed back so the current (period+peer) and previous
// (period-only) twins both resolve to real data.
function respond(req: MetricResultsRequest): MetricResultsResponse {
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
              values: req.entity.ids.map((id) => ({ entity_id: id, value: 1 })),
            }
          : {
              view: "peer",
              values: req.entity.ids.map((id) => ({
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

  it("returns the current byKey and the previous-period twin, keyed once", async () => {
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

    // Current fetch requests period + peer; the previous twin requests period
    // only (trends compare values, not standings).
    const viewsFor = (from: string) =>
      mock.mock.calls
        .map((c) => c[0])
        .find((r) => r.period.from === from)
        ?.metrics[0]?.views.map((v) => v.view);
    expect(viewsFor(RANGE.from)).toEqual(["period", "peer"]);
    // The previous range differs from the current; its twin is period-only.
    const prevCall = mock.mock.calls
      .map((c) => c[0])
      .find((r) => r.period.from !== RANGE.from);
    expect(prevCall?.metrics[0]?.views.map((v) => v.view)).toEqual(["period"]);
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
