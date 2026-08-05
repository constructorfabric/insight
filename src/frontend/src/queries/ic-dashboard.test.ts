import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getPerson } from "@/api/identity-client";
import type { IdentityPerson } from "@/types/insight";
import { useIcPerson } from "@/queries/ic-dashboard";

vi.mock("@/api/identity-client", async (orig) => ({
  ...(await orig<typeof import("@/api/identity-client")>()),
  getPerson: vi.fn(),
}));

const mockGetPerson = vi.mocked(getPerson);

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
}

beforeEach(() => {
  mockGetPerson.mockReset();
});

describe("useIcPerson", () => {
  it("resolves the identity person", async () => {
    const person = { email: "alice@x.com" } as IdentityPerson;
    mockGetPerson.mockResolvedValue(person);
    const { result } = renderHook(() => useIcPerson("Alice@X.com"), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toBe(person);
    expect(mockGetPerson).toHaveBeenCalledWith("Alice@X.com");
  });

  it("stays disabled for an empty person id", async () => {
    const { result } = renderHook(() => useIcPerson(""), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.fetchStatus).toBe("idle"));
    expect(mockGetPerson).not.toHaveBeenCalled();
  });
});
