/**
 * The roster every flat-org zone counts. What matters: the walk collects every
 * page rather than the first, it stops at a ceiling instead of following a
 * cursor forever, and a refusal surfaces as an error — an empty roster would
 * read to every zone as "this person can see nobody", which is a different
 * fact.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as identityClient from "@/api/identity-client";

vi.mock("@/api/identity-client");

const session = vi.hoisted(() => ({ value: { scope: "tenant-a" } as unknown }));
vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: session.value }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: (s: unknown) =>
    s == null ? null : (s as { scope: string }).scope,
}));

import { MAX_ROSTER_PAGES, useVisibleRoster } from "./visible-roster";

const listVisiblePersons = vi.mocked(identityClient.listVisiblePersons);

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  return { wrapper };
}

function person(id: string): identityClient.PersonSummary {
  return { person_id: id, display_name: `Person ${id}` };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useVisibleRoster", () => {
  it("collects every page, not the first", async () => {
    listVisiblePersons
      .mockResolvedValueOnce({ items: [person("a")], next_cursor: "c1" })
      .mockResolvedValueOnce({ items: [person("b")], next_cursor: "c2" })
      .mockResolvedValueOnce({ items: [person("c")], next_cursor: null });

    const { result } = renderHook(() => useVisibleRoster(true), harness());

    await waitFor(() => expect(result.current.roster).toHaveLength(3));
    expect(result.current.roster.map((p) => p.person_id)).toEqual(["a", "b", "c"]);
    expect(result.current.truncated).toBe(false);
  });

  it("stops at the ceiling rather than following a cursor forever", async () => {
    // A service that always returns a cursor must not spin the browser.
    listVisiblePersons.mockResolvedValue({
      items: [person("x")],
      next_cursor: "always-more",
    });

    const { result } = renderHook(() => useVisibleRoster(true), harness());

    await waitFor(() => expect(result.current.truncated).toBe(true));
    expect(listVisiblePersons).toHaveBeenCalledTimes(MAX_ROSTER_PAGES);
  });

  it("surfaces a refusal instead of an empty roster", async () => {
    listVisiblePersons.mockRejectedValue(new Error("refused"));

    const { result } = renderHook(() => useVisibleRoster(true), harness());

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.roster).toEqual([]);
  });

  it("asks for nothing until the caller says the policy needs it", () => {
    renderHook(() => useVisibleRoster(false), harness());

    expect(listVisiblePersons).not.toHaveBeenCalled();
  });
});
