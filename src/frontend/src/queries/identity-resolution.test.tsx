/**
 * The verb hooks' cache behavior. What matters: the accounts the SERVER
 * reported as decided leave the cached attention queue at once — under the
 * REAL query key, sessionScope suffix included, since a prefix drift here
 * ships green and the operator keeps staring at rows they already decided
 * until the slow refetch lands. A refused account keeps its row, and both
 * the resolution and person-search cache families are invalidated.
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

import { useBindAccount } from "./identity-resolution";

const bindAccount = vi.mocked(identityClient.bindAccount);

const ATTENTION_KEY = ["identity", "resolution", "attention", "tenant-a"];

const REF = {
  source: "github",
  source_id: "01900000-0000-7000-8000-00000000aa01",
  account_id: "a1",
};

function item(account_id: string): identityClient.AttentionItem {
  return {
    kind: "contested",
    source: REF.source,
    source_id: REF.source_id,
    account_id,
    email: `${account_id}@example.com`,
    username: null,
    candidates: [],
  };
}

function outcome(status: string): identityClient.CorrectionResponse {
  return {
    applied: status === "applied" ? 1 : 0,
    already_decided: 0,
    items: [{ ...REF, outcome: status }],
  };
}

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  return { queryClient, wrapper };
}

beforeEach(() => {
  vi.resetAllMocks();
  session.value = { scope: "tenant-a" };
});

describe("useBindAccount cache behavior", () => {
  it("drops the decided row from the cached queue under the real key", async () => {
    const { queryClient, wrapper } = harness();
    queryClient.setQueryData(ATTENTION_KEY, {
      items: [item("a1"), item("a2")],
      rates: { observed: 2, bound: 0, pending: 2, no_evidence: 0, excluded: 0 },
    });
    bindAccount.mockResolvedValueOnce(outcome("applied"));

    const { result } = renderHook(() => useBindAccount(), { wrapper });
    result.current.mutate({
      account: { source: REF.source, source_id: REF.source_id, id: REF.account_id },
      person_id: "01900000-0000-7000-8000-0000000000b0",
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const cached = queryClient.getQueryData<identityClient.AttentionResponse>(
      ATTENTION_KEY,
    );
    expect(cached?.items.map((i) => i.account_id)).toEqual(["a2"]);
  });

  // A refusal changed nothing on the server, so the row must survive the
  // prune — removing it would show a queue that dealt with something the
  // server declined.
  it("keeps a refused row in the cached queue", async () => {
    const { queryClient, wrapper } = harness();
    queryClient.setQueryData(ATTENTION_KEY, {
      items: [item("a1")],
      rates: { observed: 1, bound: 0, pending: 1, no_evidence: 0, excluded: 0 },
    });
    bindAccount.mockResolvedValueOnce(outcome("refused"));

    const { result } = renderHook(() => useBindAccount(), { wrapper });
    result.current.mutate({
      account: { source: REF.source, source_id: REF.source_id, id: REF.account_id },
      person_id: "01900000-0000-7000-8000-0000000000b0",
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const cached = queryClient.getQueryData<identityClient.AttentionResponse>(
      ATTENTION_KEY,
    );
    expect(cached?.items).toHaveLength(1);
  });

  it("invalidates the resolution reads and the person-search cache", async () => {
    const { queryClient, wrapper } = harness();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    bindAccount.mockResolvedValueOnce(outcome("applied"));

    const { result } = renderHook(() => useBindAccount(), { wrapper });
    result.current.mutate({
      account: { source: REF.source, source_id: REF.source_id, id: REF.account_id },
      person_id: "01900000-0000-7000-8000-0000000000b0",
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const keys = invalidate.mock.calls.map((c) => c[0]?.queryKey);
    expect(keys).toContainEqual(["identity", "resolution"]);
    expect(keys).toContainEqual(["identity", "persons", "search"]);
  });
});
