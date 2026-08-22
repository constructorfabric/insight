/**
 * The install's metric gate, at the layer every surface fetches through.
 *
 * Its own file because `navPolicy()` reads `/config.js` once and memoises the
 * answer — a per-test policy would have to defeat that cache, and a gate that
 * only works when the cache is cold is not the gate the app runs.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.stubGlobal("__INSIGHT_CONFIG__", {
  nav: { planned: ["metric:ai.*"], hide: ["metric:tasks.pickup_time"] },
});

import { queryMetricResults } from "@/api/metric-results-client";
import type { MetricCollectionConfig } from "@/lib/metrics/collection";
import { setPortalShowPlanned } from "@/lib/portal/portal-store";
import { pid } from "@/test/identity";
import { useMetricCollection } from "@/queries/metric-results";

vi.mock("@/api/metric-results-client", async (orig) => ({
  ...(await orig<typeof import("@/api/metric-results-client")>()),
  queryMetricResults: vi.fn(),
}));
vi.mock("@/api/metric-definitions-client", () => ({
  listMetricDefinitions: vi.fn(async () => ({
    metrics: [
      { metric_key: "git.commits", is_enabled: true },
      { metric_key: "ai.cost", is_enabled: true },
      { metric_key: "tasks.pickup_time", is_enabled: true },
    ],
  })),
}));

const mock = vi.mocked(queryMetricResults);

const COLLECTION: MetricCollectionConfig = {
  metrics: [
    { key: "git.commits", views: [{ view: "period" }] },
    { key: "ai.cost", views: [{ view: "period" }] },
    { key: "tasks.pickup_time", views: [{ view: "period" }] },
  ],
};
const ENTITY = { type: "person" as const, ids: [pid("ic")] };
const RANGE = { from: "2026-06-01", to: "2026-06-30" };

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
}

async function askedFor(): Promise<string[]> {
  const { result } = renderHook(
    () => useMetricCollection(COLLECTION, ENTITY, RANGE),
    { wrapper: wrapper() }
  );
  await waitFor(() => expect(result.current.isPending).toBe(false));
  return mock.mock.calls[0]![0].metrics.map((m) => m.metric_key);
}

describe("the install's metric gate", () => {
  beforeEach(() => {
    mock.mockReset();
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
            values: [{ entity_id: ENTITY.ids[0]!, value: 1 }],
          },
        ],
      })),
    }));
  });

  it("never asks for a metric the install hides, whatever the reader toggled", async () => {
    setPortalShowPlanned(true);

    expect(await askedFor()).not.toContain("tasks.pickup_time");
  });

  it("drops a planned family for a reader with planned sections off", async () => {
    setPortalShowPlanned(false);

    expect(await askedFor()).toEqual(["git.commits"]);
  });

  it("serves a planned family to a reader who asked for planned sections", async () => {
    setPortalShowPlanned(true);

    expect(await askedFor()).toEqual(["git.commits", "ai.cost"]);
  });
});
