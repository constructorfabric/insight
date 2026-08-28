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
// The catalog gate reads this; tests that do not care about it get every key.
vi.mock("@/api/metric-definitions-client", () => ({
  listMetricDefinitions: vi.fn(async () => ({ metrics: catalog })),
}));

const mock = vi.mocked(queryMetricResults);

/** What the installation's catalog offers, per test. */
let catalog: Array<{ metric_key: string; is_enabled: boolean }> = [];
function offers(...keys: string[]) {
  catalog = keys.map((metric_key) => ({ metric_key, is_enabled: true }));
}

// Echo the requested metrics/entities back as a valid response so merges and
// pairing have real data to operate on.
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
        v.view === "peer"
          ? {
              view: "peer",
              values: ids.map((id) => ({
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
              values: ids.map((id) => ({
                entity_id: id,
                value: 1,
                ...(req.compare_to ? { compare_to: 10 } : {}),
              })),
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
    offers("m", "other");
  });

  it("asks only for metrics this installation's catalog offers", async () => {
    // The backend rejects the whole request over one unknown key, so a
    // compiled-in key a tenant does not have must not reach it — otherwise a
    // single missing metric blanks the screen instead of its own tile.
    offers("m");
    const withUnknown: MetricCollectionConfig = {
      metrics: [
        { key: "m", views: [{ view: "period" }] },
        { key: "tasks.closed_non_bug", views: [{ view: "period" }] },
      ],
    };
    const { result } = renderHook(
      () => useMetricCollection(withUnknown, ENTITY, RANGE),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(mock).toHaveBeenCalledTimes(1);
    expect(mock.mock.calls[0]![0].metrics.map((m) => m.metric_key)).toEqual(["m"]);
    expect(result.current.byKey.get("m")).toBeDefined();
    expect(result.current.byKey.get("tasks.closed_non_bug")).toBeUndefined();
  });

  it("makes no request at all when the catalog offers none of the collection", async () => {
    // An empty `metrics: []` is itself a 400, and a screen must not sit on a
    // spinner waiting for a request that will never be sent.
    offers("something.else");
    const { result } = renderHook(
      () => useMetricCollection(COLLECTION, ENTITY, RANGE),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(mock).not.toHaveBeenCalled();
    expect(result.current.isError).toBe(false);
  });

  it("normalizes the current result and asks for no comparison by default", async () => {
    const { result } = renderHook(
      () => useMetricCollection(COLLECTION, ENTITY, RANGE),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(result.current.byKey.get("m")?.period?.values).toHaveLength(1);
    expect(result.current.previousByKey).toBeNull();
    expect(mock).toHaveBeenCalledTimes(1);
  });

  it("serves the previous period as the comparison window of one request", async () => {
    const { result } = renderHook(
      () => useMetricCollection(COLLECTION, ENTITY, RANGE, { previousPeriod: "month" }),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.isPending).toBe(false));

    expect(mock).toHaveBeenCalledTimes(1);
    expect(mock.mock.calls[0]![0].compare_to).toEqual({
      from: "2026-05-01",
      to: "2026-05-30",
    });
    expect(
      result.current.previousByKey?.get("m")?.period?.values[0]?.value,
    ).toBe(10);
  });

  it("reads an explicit comparison window back through previousByKey", async () => {
    const compareTo = { from: "2026-05-01", to: "2026-05-15" };
    const { result } = renderHook(
      () => useMetricCollection(COLLECTION, ENTITY, RANGE, { compareTo }),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.isPending).toBe(false));

    expect(mock).toHaveBeenCalledTimes(1);
    expect(mock.mock.calls[0]![0].compare_to).toEqual(compareTo);
    expect(result.current.previousByKey?.get("m")?.period?.values[0]?.value).toBe(10);
    // The window carries no standing: reading `peer` here would answer over the
    // primary period instead.
    expect(result.current.previousByKey?.get("m")?.peer).toBeUndefined();
  });

  it("surfaces errors and leaves byKey empty", async () => {
    mock.mockRejectedValue(new Error("request failed"));
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
    offers("m", "other");
  });

  it("holds every request until the catalog answers, then drops unknown keys", async () => {
    offers("m");
    const { result } = renderHook(
      () =>
        useMetricCollectionSet(
          [
            { key: "g", collection: COLLECTION },
            {
              key: "unavailable",
              collection: {
                metrics: [{ key: "nope", views: [{ view: "period" }] }],
              },
            },
          ],
          ENTITY,
          RANGE,
        ),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(collectionSetPending(result.current)).toBe(false));
    // One request for the collection the catalog covers; none for the other,
    // which would have failed BOTH of them as a single 400.
    expect(mock).toHaveBeenCalledTimes(1);
    expect(mock.mock.calls[0]![0].metrics.map((m) => m.metric_key)).toEqual(["m"]);
    expect(result.current.get("g")?.byKey.get("m")).toBeDefined();
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

  it("serves a tenant entity as one unchunked id-less request", async () => {
    // The tenant lens fans its extra collections through this hook; a tenant
    // names nobody, so the person-roster gates must not keep it disabled.
    mock.mockImplementation(async (req) => ({
      metrics: req.metrics.map((m) => ({
        metric_key: m.metric_key,
        label: m.metric_key,
        unit: null,
        format: "integer" as const,
        direction: "higher_is_better" as const,
        computation: "sum" as const,
        views: [
          {
            view: "period" as const,
            values: [{ entity_id: "tenant-1", value: 7 }],
          },
        ],
      })),
    }));
    const { result } = renderHook(
      () =>
        useMetricCollectionSet(
          [
            {
              key: "g",
              collection: {
                metrics: [{ key: "m", views: [{ view: "period" }] }],
              },
            },
          ],
          { type: "tenant" },
          RANGE,
        ),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(result.current.get("g")?.isPending).toBe(false));
    expect(mock).toHaveBeenCalledTimes(1);
    expect(mock.mock.calls[0]![0].entity).toEqual({ type: "tenant" });
    expect(
      result.current.get("g")?.byKey.get("m")?.period?.values,
    ).toEqual([{ entity_id: "tenant-1", value: 7 }]);
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

/**
 * Reported from the review environment: `/v1/metric-results` answering 400
 * `entity.ids must not be empty`. The request was ours — `refetch()` ignores
 * `enabled`, so a Retry on a view whose roster had not resolved posted an empty
 * entity list, and the resulting error then latched the view into a hard-error
 * card that no further Retry could clear.
 */
describe("a disabled collection has nothing to retry", () => {
  beforeEach(() => {
    mock.mockReset();
    mock.mockImplementation((req: MetricResultsRequest) =>
      Promise.resolve(respond(req)),
    );
  });

  it("sends nothing while the entity list is empty", () => {
    renderHook(
      () => useMetricCollection(COLLECTION, { type: "person", ids: [] }, RANGE),
      { wrapper: wrapper() },
    );
    expect(mock).not.toHaveBeenCalled();
  });

  it("ignores refetch instead of posting an entity-less request", async () => {
    const { result } = renderHook(
      () => useMetricCollection(COLLECTION, { type: "person", ids: [] }, RANGE),
      { wrapper: wrapper() },
    );

    result.current.refetch();

    // A short settle window: the failure mode is a request that DOES go out.
    await new Promise((r) => setTimeout(r, 20));
    expect(mock).not.toHaveBeenCalled();
  });

  it("ignores refetch on a set of chunked collections too", async () => {
    const { result } = renderHook(
      () =>
        useMetricCollectionSet(
          [{ key: "g", collection: COLLECTION }],
          { type: "person", ids: [] },
          RANGE,
        ),
      { wrapper: wrapper() },
    );

    result.current.get("g")?.refetch();

    await new Promise((r) => setTimeout(r, 20));
    expect(mock).not.toHaveBeenCalled();
  });

  it("still retries once there is something to ask for", async () => {
    const { result } = renderHook(
      () =>
        useMetricCollection(COLLECTION, { type: "person", ids: ["a@x"] }, RANGE),
      { wrapper: wrapper() },
    );
    await waitFor(() => expect(mock).toHaveBeenCalledTimes(1));

    result.current.refetch();
    await waitFor(() => expect(mock).toHaveBeenCalledTimes(2));
  });

  it("does not carry a failure across into a resolved roster", async () => {
    // The failure was recorded against the entity list that produced it (ids
    // ride in the query key), so once the roster resolves the view starts from
    // a clean query rather than inheriting "Unable to load".
    mock.mockRejectedValue(new Error("400 invalid_argument"));
    const { result, rerender } = renderHook(
      ({ ids }: { ids: string[] }) =>
        useMetricCollection(COLLECTION, { type: "person", ids }, RANGE),
      { wrapper: wrapper(), initialProps: { ids: ["a@x"] } },
    );
    await waitFor(() => expect(result.current.isError).toBe(true));

    mock.mockImplementation((req: MetricResultsRequest) =>
      Promise.resolve(respond(req)),
    );
    rerender({ ids: ["a@x", "b@x"] });
    await waitFor(() => expect(result.current.isError).toBe(false));
  });
});
