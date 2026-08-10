import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as client from "@/api/metrics-client";
import * as resultsClient from "@/api/metric-results-client";

import {
  useCreateCustomMetric,
  useCustomMetric,
  useCustomMetricPreview,
  useCustomMetrics,
  useDeleteCustomMetric,
  useUpdateCustomMetric,
} from "./custom-metrics";

vi.mock("@/api/metrics-client");
vi.mock("@/api/metric-results-client");

const GRAPH: client.CustomMetricGraph = {
  metric_key: "example.lines",
  label: "Lines",
  short_label: null,
  description: null,
  explanation: null,
  entity_type: "person",
  unit: "lines",
  format: "integer",
  direction: "higher_is_better",
  computation: "sum",
  scale: null,
  peer_cohort_key: null,
  transform: null,
  source_key: "example_source",
  observation_sql: "SELECT 1",
  measures: ["lines"],
  dimensions: ["repo"],
  inputs: [{ role: "value", measure_key: "lines" }],
};

const METRIC: client.CustomMetric = { ...GRAPH, origin: "custom" };

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  return { queryClient, wrapper };
}

beforeEach(() => vi.resetAllMocks());

describe("useCustomMetrics", () => {
  it("selects the items array from the list response", async () => {
    vi.mocked(client.listCustomMetrics).mockResolvedValue({
      items: [
        {
          metric_key: "example.lines",
          label: "Lines",
          computation: "sum",
          entity_type: "person",
        },
      ],
    });
    const { wrapper } = harness();
    const { result } = renderHook(() => useCustomMetrics(), { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([
      {
        metric_key: "example.lines",
        label: "Lines",
        computation: "sum",
        entity_type: "person",
      },
    ]);
  });
});

describe("useCustomMetric", () => {
  it("is disabled (idle) for a null key and never fetches", () => {
    const { wrapper } = harness();
    const { result } = renderHook(() => useCustomMetric(null), { wrapper });
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getCustomMetric).not.toHaveBeenCalled();
  });

  it("fetches by key when enabled", async () => {
    vi.mocked(client.getCustomMetric).mockResolvedValue(METRIC);
    const { wrapper } = harness();
    const { result } = renderHook(() => useCustomMetric("example.lines"), {
      wrapper,
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(client.getCustomMetric).toHaveBeenCalledWith("example.lines");
  });
});

describe("mutations", () => {
  it("create calls the client and invalidates the list", async () => {
    vi.mocked(client.createCustomMetric).mockResolvedValue(METRIC);
    const { queryClient, wrapper } = harness();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useCreateCustomMetric(), { wrapper });

    result.current.mutate(GRAPH);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(client.createCustomMetric).toHaveBeenCalledWith(GRAPH);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["custom-metrics"] });
  });

  it("update calls the client and seeds the detail cache", async () => {
    vi.mocked(client.updateCustomMetric).mockResolvedValue(METRIC);
    const { queryClient, wrapper } = harness();
    const setData = vi.spyOn(queryClient, "setQueryData");
    const { result } = renderHook(
      () => useUpdateCustomMetric("example.lines"),
      { wrapper }
    );

    result.current.mutate(GRAPH);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(client.updateCustomMetric).toHaveBeenCalledWith(
      "example.lines",
      GRAPH
    );
    expect(setData).toHaveBeenCalledWith(
      ["custom-metrics", "example.lines"],
      METRIC
    );
  });

  it("delete calls the client with the metric key", async () => {
    vi.mocked(client.deleteCustomMetric).mockResolvedValue(undefined);
    const { wrapper } = harness();
    const { result } = renderHook(() => useDeleteCustomMetric(), { wrapper });

    result.current.mutate("example.lines");
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(client.deleteCustomMetric).toHaveBeenCalledWith("example.lines");
  });
});

describe("useCustomMetricPreview", () => {
  it("posts a single period view to the results endpoint by metric key", async () => {
    vi.mocked(resultsClient.queryMetricResults).mockResolvedValue({
      metrics: [],
    });
    const { wrapper } = harness();
    const { result } = renderHook(
      () => useCustomMetricPreview("example.lines"),
      { wrapper }
    );

    result.current.mutate({
      entityType: "person",
      entityIds: ["00000000-0000-0000-0000-0000000000aa"],
      from: "2026-01-01",
      to: "2026-01-31",
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(resultsClient.queryMetricResults).toHaveBeenCalledWith({
      entity: {
        type: "person",
        ids: ["00000000-0000-0000-0000-0000000000aa"],
      },
      period: { from: "2026-01-01", to: "2026-01-31" },
      metrics: [
        { metric_key: "example.lines", views: [{ view: "period" }] },
      ],
    });
  });
});
