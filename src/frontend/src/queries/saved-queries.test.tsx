import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as client from "@/api/saved-queries-client";

import {
  useCreateSavedQuery,
  useDeleteSavedQuery,
  useRunSavedQuery,
  useSavedQueries,
  useSavedQuery,
  useUpdateSavedQuery,
} from "./saved-queries";

vi.mock("@/api/saved-queries-client");

const SAVED: client.SavedQuery = {
  id: "q-1",
  insight_tenant_id: "t-1",
  name: "Q",
  description: null,
  sql: "SELECT 1",
  created_at: "2026-07-01T00:00:00Z",
  updated_at: "2026-07-01T00:00:00Z",
};

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

describe("useSavedQueries", () => {
  it("selects the items array from the list response", async () => {
    vi.mocked(client.listSavedQueries).mockResolvedValue({
      items: [{ id: "q-1", name: "Q", description: null }],
    });
    const { wrapper } = harness();
    const { result } = renderHook(() => useSavedQueries(), { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([
      { id: "q-1", name: "Q", description: null },
    ]);
  });
});

describe("useSavedQuery", () => {
  it("is disabled (idle) for a null id and never fetches", () => {
    const { wrapper } = harness();
    const { result } = renderHook(() => useSavedQuery(null), { wrapper });
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getSavedQuery).not.toHaveBeenCalled();
  });

  it("fetches by id when enabled", async () => {
    vi.mocked(client.getSavedQuery).mockResolvedValue(SAVED);
    const { wrapper } = harness();
    const { result } = renderHook(() => useSavedQuery("q-1"), { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(client.getSavedQuery).toHaveBeenCalledWith("q-1");
  });
});

describe("mutations", () => {
  it("create calls the client and invalidates the list", async () => {
    vi.mocked(client.createSavedQuery).mockResolvedValue(SAVED);
    const { queryClient, wrapper } = harness();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useCreateSavedQuery(), { wrapper });

    result.current.mutate({ name: "Q", sql: "SELECT 1" });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(client.createSavedQuery).toHaveBeenCalledWith({
      name: "Q",
      sql: "SELECT 1",
    });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["saved-queries"] });
  });

  it("update calls the client and seeds the detail cache", async () => {
    vi.mocked(client.updateSavedQuery).mockResolvedValue(SAVED);
    const { queryClient, wrapper } = harness();
    const setData = vi.spyOn(queryClient, "setQueryData");
    const { result } = renderHook(() => useUpdateSavedQuery("q-1"), {
      wrapper,
    });

    result.current.mutate({ name: "R" });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(client.updateSavedQuery).toHaveBeenCalledWith("q-1", { name: "R" });
    expect(setData).toHaveBeenCalledWith(["saved-queries", "q-1"], SAVED);
  });

  it("delete calls the client with the id", async () => {
    vi.mocked(client.deleteSavedQuery).mockResolvedValue(undefined);
    const { wrapper } = harness();
    const { result } = renderHook(() => useDeleteSavedQuery(), { wrapper });

    result.current.mutate("q-1");
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(client.deleteSavedQuery).toHaveBeenCalledWith("q-1");
  });

  it("run calls the client with the id and body", async () => {
    vi.mocked(client.runSavedQuery).mockResolvedValue({ rows: [] });
    const { wrapper } = harness();
    const { result } = renderHook(() => useRunSavedQuery("q-1"), { wrapper });

    result.current.mutate({ period: "2026-01" });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(client.runSavedQuery).toHaveBeenCalledWith("q-1", {
      period: "2026-01",
    });
  });
});
