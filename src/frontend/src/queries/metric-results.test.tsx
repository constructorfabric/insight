import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  MetricResultsRequest,
  MetricResultsResponse,
} from "@/api/metric-results-client";
import { queryMetricResults } from "@/api/metric-results-client";
import type { MetricCollectionConfig } from "@/lib/metrics/collection";
import {
  collectionSetPending,
  useMetricCollection,
  useMetricCollectionSet,
  type MetricCollectionResult,
} from "@/queries/metric-results";

vi.mock("@/api/metric-results-client", async (orig) => ({
  ...(await orig<typeof import("@/api/metric-results-client")>()),
  queryMetricResults: vi.fn(),
}));

const mock = vi.mocked(queryMetricResults);

// Echo the requested metrics/entities back as a valid response so merges and
// pairing have real data to operate on.
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
        v.view === "peer"
          ? {
              view: "peer",
              values: req.entity.ids.map((id) => ({
                entity_id: id,
                target_value: 1,
                p25: 0,
                median: 1,
                p75: 2,
                min: 0,
                max: 3,
                n: 10,
              })),
            }
          : {
              view: "period",
              values: req.entity.ids.map((id) => ({ entity_id: id, value: 1 })),
            },
      ),
    })),
  };
}

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
}

const COLLECTION: MetricCollectionConfig = {
  metrics: [{ key: "m", views: [{ view: "period" }, { view: "peer" }] }],
};
const ENTITY = { type: "person" as const, ids: ["me@x.com"] };
const RANGE = { from: "2026-06-01", to: "2026-06-30" };

describe("useMetricCollection", () => {
  beforeEach(() => {
    mock.mockReset();
    mock.mockImplementation(async (req) => respond(req));
  });

  it("normalizes the current result and skips the previous twin by default", async () => {
    const { result } = renderHook(
      () => useMetricCollection(COLLECTION, ENTITY, RANGE),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(result.current.byKey.get("m")?.period?.values).toHaveLength(1);
    expect(result.current.previousByKey).toBeNull();
    expect(mock).toHaveBeenCalledTimes(1);
  });

  it("fires the previous-period twin and exposes previousByKey", async () => {
    const { result } = renderHook(
      () => useMetricCollection(COLLECTION, ENTITY, RANGE, { previousPeriod: "month" }),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.previousByKey).not.toBeNull());
    expect(result.current.previousByKey?.get("m")).toBeDefined();
    expect(mock).toHaveBeenCalledTimes(2);
  });

  it("surfaces errors and leaves byKey empty", async () => {
    mock.mockRejectedValue(new Error("boom"));
    const { result } = renderHook(
      () => useMetricCollection(COLLECTION, ENTITY, RANGE),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.byKey.size).toBe(0);
  });

  it("does not fetch for an empty entity set", async () => {
    const { result } = renderHook(
      () => useMetricCollection(COLLECTION, { type: "person", ids: [] }, RANGE),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(mock).not.toHaveBeenCalled();
  });
});

describe("useMetricCollectionSet", () => {
  beforeEach(() => {
    mock.mockReset();
    mock.mockImplementation(async (req) => respond(req));
  });

  it("splits a large roster into chunks and merges them into one result", async () => {
    const ids = Array.from({ length: 3000 }, (_, i) => `p${i}@x.com`);
    const { result } = renderHook(
      () =>
        useMetricCollectionSet(
          [{ key: "g", collection: COLLECTION }],
          { type: "person", ids },
          RANGE,
        ),
      { wrapper: wrapper() },
    );
    await waitFor(() =>
      expect(result.current.get("g")?.isPending).toBe(false),
    );
    // period+peer → 2 rows/entity → 2250/chunk → 3000 ids span 2 requests.
    expect(mock).toHaveBeenCalledTimes(2);
    expect(result.current.get("g")?.byKey.get("m")?.period?.values).toHaveLength(
      3000,
    );
  });

  it("aggregates refetch across a collection's chunks", async () => {
    const ids = Array.from({ length: 3000 }, (_, i) => `p${i}@x.com`);
    const { result } = renderHook(
      () =>
        useMetricCollectionSet(
          [{ key: "g", collection: COLLECTION }],
          { type: "person", ids },
          RANGE,
        ),
      { wrapper: wrapper() },
    );
    await waitFor(() =>
      expect(result.current.get("g")?.isPending).toBe(false),
    );
    mock.mockClear();
    result.current.get("g")?.refetch();
    await waitFor(() => expect(mock).toHaveBeenCalledTimes(2));
  });
});

describe("collectionSetPending", () => {
  const r = (isPending: boolean): MetricCollectionResult => ({
    byKey: new Map(),
    previousByKey: null,
    isPending,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
  });
  it("is true when any collection in the set still has no data", () => {
    expect(collectionSetPending(new Map([["a", r(false)], ["b", r(true)]]))).toBe(true);
  });
  it("is false when every collection has settled", () => {
    expect(collectionSetPending(new Map([["a", r(false)], ["b", r(false)]]))).toBe(false);
    expect(collectionSetPending(new Map())).toBe(false);
  });
});
