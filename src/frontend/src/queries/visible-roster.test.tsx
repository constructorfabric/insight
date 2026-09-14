/**
 * The roster every flat-org zone counts. What matters: the walk collects every
 * page rather than the first, rejects a cursor cycle instead of looping, and a
 * refusal surfaces as an error — an empty roster would
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

import { useVisibleRoster } from "./visible-roster";

const listPeople = vi.mocked(identityClient.listPeople);

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  return { wrapper };
}

function person(id: string): identityClient.PeopleListItem {
  return {
    person_id: id,
    display_name: `Person ${id}`,
    first_name: null,
    last_name: null,
    username: null,
    email: null,
    attributes: {},
    manager_person_id: null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useVisibleRoster", () => {
  it("collects every page until the service ends the cursor walk", async () => {
    listPeople
      .mockResolvedValueOnce({ items: [person("a")], next_cursor: "c1" })
      .mockResolvedValueOnce({ items: [person("b")], next_cursor: "c2" })
      .mockResolvedValueOnce({ items: [person("c")], next_cursor: "c3" })
      .mockResolvedValueOnce({ items: [person("d")], next_cursor: "c4" })
      .mockResolvedValueOnce({ items: [person("e")], next_cursor: null });

    const { result } = renderHook(() => useVisibleRoster(true), harness());

    await waitFor(() => expect(result.current.roster).toHaveLength(5));
    expect(result.current.roster.map((p) => p.person_id)).toEqual([
      "a",
      "b",
      "c",
      "d",
      "e",
    ]);
  });

  it("rejects a repeated cursor rather than following it forever", async () => {
    listPeople.mockResolvedValue({
      items: [person("x")],
      next_cursor: "always-more",
    });

    const { result } = renderHook(() => useVisibleRoster(true), harness());

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(listPeople).toHaveBeenCalledTimes(2);
  });

  it("forwards query cancellation to every people request", async () => {
    listPeople.mockResolvedValue({ items: [], next_cursor: null });

    const { result } = renderHook(() => useVisibleRoster(true), harness());

    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(listPeople.mock.calls[0]?.[1]).toBeInstanceOf(AbortSignal);
  });

  it("surfaces a refusal instead of an empty roster", async () => {
    listPeople.mockRejectedValue(new Error("refused"));

    const { result } = renderHook(() => useVisibleRoster(true), harness());

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.roster).toEqual([]);
  });

  it("asks for nothing until the caller says the policy needs it", () => {
    renderHook(() => useVisibleRoster(false), harness());

    expect(listPeople).not.toHaveBeenCalled();
  });
});
