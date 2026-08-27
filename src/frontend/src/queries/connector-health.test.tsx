/**
 * The two hooks' contracts: the per-connector window waits for a connector
 * rather than firing with a placeholder, and neither hands back a cached answer
 * across sessions.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const session = vi.hoisted(() => ({ value: "scope-a" as string }));
vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: session.value }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: (value: unknown) => value as string,
}));
vi.mock("@/api/connector-health-client", () => ({
  getConnectorHealth: vi.fn(),
  getConnectorSyncs: vi.fn(),
}));

import * as client from "@/api/connector-health-client";
import {
  useConnectorHealth,
  useConnectorSyncs,
} from "@/queries/connector-health";

const mockSummary = vi.mocked(client.getConnectorHealth);
const mockSyncs = vi.mocked(client.getConnectorSyncs);

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: qc }, children);
}

const SUMMARY = {
  as_of: "2026-01-15T12:00:00.000Z",
  checked_at: "2026-01-15T11:59:00.000Z",
  typical_read_interval_ms: 900_000,
  history_available: true,
  connectors: [],
};

beforeEach(() => {
  mockSummary.mockReset();
  mockSyncs.mockReset();
  session.value = "scope-a";
});

describe("useConnectorHealth", () => {
  it("serves the recorded summary", async () => {
    mockSummary.mockResolvedValue(SUMMARY);

    const { result } = renderHook(() => useConnectorHealth(), {
      wrapper: wrapper(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual(SUMMARY);
  });

  it("surfaces a failure rather than an empty page", async () => {
    mockSummary.mockRejectedValue(new Error("refused"));

    const { result } = renderHook(() => useConnectorHealth(), {
      wrapper: wrapper(),
    });

    await waitFor(() => expect(result.current.isError).toBe(true));
  });
});

describe("one session's answer is never served to another", () => {
  it("caches under a key that carries the authorization scope", async () => {
    // The surface is instance-wide and operator-gated, so a cached answer
    // crossing a session boundary would show one caller what another was
    // allowed to see.
    //
    // Asserted on the KEY, not on a refetch count: both hooks set
    // `staleTime: 0`, so a second observer refetches whatever the key is — a
    // fetch-count test passes with the scope removed from the key entirely,
    // which is exactly the regression it would exist to catch.
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: Infinity } },
    });
    const wrap = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client }, children);

    mockSummary.mockResolvedValue(SUMMARY);
    const first = renderHook(() => useConnectorHealth(), { wrapper: wrap });
    await waitFor(() => expect(first.result.current.isSuccess).toBe(true));

    session.value = "scope-b";
    const second = renderHook(() => useConnectorHealth(), { wrapper: wrap });
    await waitFor(() => expect(second.result.current.isSuccess).toBe(true));

    const keys = client
      .getQueryCache()
      .getAll()
      .map((query) => JSON.stringify(query.queryKey));
    expect(new Set(keys).size).toBe(2);
    expect(keys.some((key) => key.includes("scope-a"))).toBe(true);
    expect(keys.some((key) => key.includes("scope-b"))).toBe(true);
  });

  it("keys the per-connector window by scope as well", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: Infinity } },
    });
    const wrap = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client }, children);

    mockSyncs.mockResolvedValue({ connector: "alpha", syncs: [], window: 50 });
    const first = renderHook(() => useConnectorSyncs("alpha"), { wrapper: wrap });
    await waitFor(() => expect(first.result.current.isSuccess).toBe(true));

    session.value = "scope-b";
    const second = renderHook(() => useConnectorSyncs("alpha"), { wrapper: wrap });
    await waitFor(() => expect(second.result.current.isSuccess).toBe(true));

    const keys = client
      .getQueryCache()
      .getAll()
      .map((query) => JSON.stringify(query.queryKey));
    expect(new Set(keys).size).toBe(2);
  });
});

describe("useConnectorSyncs", () => {
  it("does not ask for a window until a connector is chosen", () => {
    const { result } = renderHook(() => useConnectorSyncs(null), {
      wrapper: wrapper(),
    });

    expect(mockSyncs).not.toHaveBeenCalled();
    expect(result.current.fetchStatus).toBe("idle");
  });

  it("reads the chosen connector's window", async () => {
    const history = { connector: "alpha", syncs: [], window: 50 };
    mockSyncs.mockResolvedValue(history);

    const { result } = renderHook(() => useConnectorSyncs("alpha"), {
      wrapper: wrapper(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockSyncs).toHaveBeenCalledWith("alpha");
    expect(result.current.data).toEqual(history);
  });
});
