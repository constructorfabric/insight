/**
 * Coverage for everyone the viewer may see.
 *
 * Two of these tests are about the request rather than the answer, because
 * both ways this hook can go wrong are in the request: asking for an id the
 * viewer may not see refuses the whole screen, and asking for a view that
 * cannot be chunked breaks it at roster scale. Neither failure is visible in
 * the returned shape, so neither would be caught by testing the output.
 */
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MetricCollectionConfig } from "@/lib/metrics/collection";

const state = vi.hoisted(() => ({
  members: [] as string[],
  definitions: undefined as unknown,
  definitionsError: false,
  collectionSet: new Map<string, unknown>(),
  lastCall: null as {
    collections: readonly { key: string; collection: MetricCollectionConfig }[];
    entity: { type: string; ids: string[] };
  } | null,
}));

vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    dateRange: { from: "2026-03-01", to: "2026-03-31" },
  }),
}));
vi.mock("@/queries/metric-definitions", () => ({
  useMetricDefinitionsResponse: () => ({
    data: state.definitions,
    isPending: false,
    isError: state.definitionsError,
  }),
}));
vi.mock("@/queries/metric-results", () => ({
  useMetricCollectionSet: (
    collections: readonly {
      key: string;
      collection: MetricCollectionConfig;
    }[],
    entity: { type: string; ids: string[] },
  ) => {
    state.lastCall = { collections, entity };
    return state.collectionSet;
  },
}));

import { GROUPS } from "@/lib/insight/groups";
import { useScopeCoverage } from "./use-scope-coverage";

beforeEach(() => {
  state.members = ["viewer-1", "a-1", "a-2", "a-3"];
  state.definitions = { metrics: [] };
  state.definitionsError = false;
  state.collectionSet = new Map();
  state.lastCall = null;
});

describe("useScopeCoverage", () => {
  it("asks for exactly the members the scope selector gave it", () => {
    // The visibility check on the metrics endpoint is all-or-nothing: a single
    // id the caller may not see refuses the entire request rather than
    // filtering it, and does not say which id was at fault. Widening or
    // guessing the list therefore empties the screen with no diagnosis.
    renderHook(() => useScopeCoverage(state.members));

    expect(state.lastCall?.entity.type).toBe("person");
    expect([...(state.lastCall?.entity.ids ?? [])].sort()).toEqual([
      "a-1",
      "a-2",
      "a-3",
      "viewer-1",
    ]);
  });

  it("requests only the period view, so the roster can still be chunked", () => {
    // `entityChunkSize` refuses to chunk a collection carrying timeseries,
    // breakdown or histogram views, and an unchunked roster-sized request runs
    // into the backend's projected-row limit. Asking for one view keeps the
    // existing chunk-and-merge path available.
    renderHook(() => useScopeCoverage(state.members));

    const views = (state.lastCall?.collections ?? []).flatMap((c) =>
      c.collection.metrics.flatMap((m) => m.views.map((v) => v.view)),
    );
    expect(views.length).toBeGreaterThan(0);
    expect([...new Set(views)]).toEqual(["period"]);
  });

  it("counts every person in the roster, including the viewer", () => {
    const { result } = renderHook(() => useScopeCoverage(state.members));
    expect(result.current.distribution.counted).toBe(4);
    expect(result.current.people).toHaveLength(4);
  });

  it("settles rather than waiting forever on an empty scope", () => {
    // No members means no collections are sent, so no group ever reports a
    // pending state to clear. Reading that as "still loading" leaves the
    // section on its spinner permanently instead of saying the scope is empty.
    state.members = [];
    const { result } = renderHook(() => useScopeCoverage(state.members));
    expect(result.current.isPending).toBe(false);
    expect(result.current.distribution.counted).toBe(0);
  });

  it("stays closed while the scope has not resolved", () => {
    // An empty id list is refused by the client rather than sent, so the hook
    // must not reach that path at all before the roster answers.
    state.members = [];
    renderHook(() => useScopeCoverage(state.members));
    expect(state.lastCall?.entity.ids).toEqual([]);
    expect(state.lastCall?.collections).toEqual([]);
  });

  it("reports every part as unreachable when nothing has ever observed", () => {
    // No definition has observed anything, so no part can be claimed to reach
    // us — and this is read from the listing, never from the roster's nulls.
    const { result } = renderHook(() => useScopeCoverage(state.members));
    expect(result.current.parts.every((p) => p.unreachable)).toBe(true);
    expect(result.current.parts).toHaveLength(GROUPS.length);
    expect(result.current.distribution.byLevel.get(0)).toBe(4);
  });
});

describe("useScopeCoverage failure", () => {
  it("reports an error rather than letting it read as an absence verdict", () => {
    // With the listing unavailable nothing is known to reach the tenant, so
    // every part would come back "no data reaches us" and every person would
    // sit at zero. That is our fault printed as a verdict about named people,
    // and the caller has to be able to tell the two apart.
    state.definitionsError = true;
    const { result } = renderHook(() => useScopeCoverage(state.members));
    expect(result.current.isError).toBe(true);
  });

  it("is not in error when everything answered", () => {
    const { result } = renderHook(() => useScopeCoverage(state.members));
    expect(result.current.isError).toBe(false);
  });
});
