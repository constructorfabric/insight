import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as identityClient from "@/api/identity-client";

vi.mock("@/api/identity-client");
vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: { scope: "tenant-a" } }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: (s: unknown) =>
    s == null ? null : (s as { scope: string }).scope,
}));

import { useDirectoryNames } from "./directory-names";

const listPeople = vi.mocked(identityClient.listPeople);

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  return { wrapper };
}

function directoryEntry(
  id: string,
  displayName: string | null,
  email: string | null = null,
): identityClient.PeopleListItem {
  return {
    person_id: id,
    display_name: displayName,
    first_name: null,
    last_name: null,
    username: null,
    email,
    attributes: {},
    manager_person_id: null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useDirectoryNames", () => {
  it("names each person the tenant directory returns, keyed by lowercase id", async () => {
    listPeople.mockResolvedValue({
      items: [
        directoryEntry("AAAA-1", "Chief Example"),
        directoryEntry("bbbb-2", null, "lead@example.com"),
      ],
      next_cursor: null,
    });

    const { result } = renderHook(
      () => useDirectoryNames(["AAAA-1", "bbbb-2"]),
      harness(),
    );

    await waitFor(() => expect(result.current.size).toBe(2));
    expect(result.current.get("aaaa-1")).toBe("Chief Example");
    expect(result.current.get("bbbb-2")).toBe("lead@example.com");
    expect(listPeople).toHaveBeenCalledWith(
      { visibility: "tenant", ids: ["AAAA-1", "bbbb-2"], limit: 2 },
      expect.any(AbortSignal),
    );
  });

  it("asks nothing when no one needs a name", () => {
    const { result } = renderHook(() => useDirectoryNames([]), harness());

    expect(result.current.size).toBe(0);
    expect(listPeople).not.toHaveBeenCalled();
  });

  it("names no one when the directory refuses", async () => {
    listPeople.mockRejectedValue(new Error("refused"));

    const { result } = renderHook(
      () => useDirectoryNames(["aaaa-1"]),
      harness(),
    );

    await waitFor(() => expect(listPeople).toHaveBeenCalled());
    expect(result.current.size).toBe(0);
  });
});
