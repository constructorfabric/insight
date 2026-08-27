/**
 * The two hooks' contracts: the per-connector window waits for a connector
 * rather than firing with a placeholder, and neither hands back a cached answer
 * across sessions.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: null }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: () => "scope",
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
