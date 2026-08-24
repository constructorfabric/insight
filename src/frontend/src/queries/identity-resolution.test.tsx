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

import {
  listsAnyAccount,
  useAttention,
  listsAnyone,
  useAccountList,
  useBindAccount,
  usePersonList,
} from "./identity-resolution";

const searchPersons = vi.mocked(identityClient.searchPersons);

const bindAccount = vi.mocked(identityClient.bindAccount);

// The key `useAttention` really writes, limit segment included — a cache entry
// under a shorter key would prove nothing about the surgery reaching the queue
// an operator is looking at.
const ATTENTION_KEY = [
  "identity",
  "resolution",
  "attention",
  "tenant-a",
  identityClient.QUEUE_FIRST_PAGE,
];

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
      rates: { persons: 2, observed: 2, bound: 0, pending: 2, no_source_id: 0, no_evidence: 0, excluded: 0 },
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
      rates: { persons: 1, observed: 1, bound: 0, pending: 1, no_source_id: 0, no_evidence: 0, excluded: 0 },
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

describe("usePersonList", () => {
  const searchPersons = vi.mocked(identityClient.searchPersons);

  function roster(): identityClient.PersonSearchResponse {
    return { items: [{ person_id: "01900000-0000-7000-8000-0000000000b0" }] };
  }

  it.each([
    ["browse", true],
    ["match", false],
  ] as const)(
    "a blank query asks for the roster under %s intent: %s",
    async (intent, asks) => {
      const { wrapper } = harness();
      searchPersons.mockResolvedValue(roster());

      const { result } = renderHook(() => usePersonList("", intent), { wrapper });

      if (asks) {
        await waitFor(() => expect(result.current.isSuccess).toBe(true));
      }
      expect(searchPersons.mock.calls.length > 0).toBe(asks);
    },
  );

  // The bug this key exists to prevent: `enabled: false` stops a request and
  // NOT a cache read, so a shared key let the dialog's picker render the roster
  // the person mode had just cached — the tenant listed into a dropdown.
  it("does not serve the browsed roster to a match-intent caller", async () => {
    const { queryClient, wrapper } = harness();
    searchPersons.mockResolvedValue(roster());
    const browsed = renderHook(() => usePersonList("", "browse"), { wrapper });
    await waitFor(() => expect(browsed.result.current.isSuccess).toBe(true));

    const matching = renderHook(() => usePersonList("", "match"), { wrapper });

    expect(matching.result.current.data).toBeUndefined();
    expect(matching.result.current.hasNextPage).toBe(false);
    expect(
      queryClient.getQueryData(["identity", "persons", "search", "tenant-a", "browse", ""]),
    ).toBeDefined();
  });

  it("pages with the cursor the service returned", async () => {
    const { wrapper } = harness();
    searchPersons
      .mockResolvedValueOnce({ ...roster(), next_cursor: "c1" })
      .mockResolvedValueOnce(roster());

    const { result } = renderHook(() => usePersonList("iva", "match"), { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.hasNextPage).toBe(true);
    await result.current.fetchNextPage();

    expect(searchPersons.mock.calls.map((c) => c[1]?.cursor)).toEqual([
      undefined,
      "c1",
    ]);
  });

  it.each([
    ["browse", "", true],
    ["browse", "   ", true],
    ["match", "", false],
    ["match", "   ", false],
    ["match", "iva", true],
    // Too short to be worth a pass over the journal — in either intent, and
    // whichever side of the debounce the value came from.
    ["browse", "i", false],
    ["browse", "iv", true],
    ["match", "i", false],
    ["match", "iv", true],
    // Trimmed before it is measured, so surrounding space neither buys nor
    // costs a search.
    ["match", "i ", false],
    ["match", " iva ", true],
    // THE FLOOR IS PER TERM, because the service matches each one against the
    // journal on its own. Measuring the field instead would read `a b` as three
    // characters and wave through two one-character passes.
    ["match", "a b", false],
    ["match", "iva b", false],
    ["match", "iv an", true],
    // Counted as the person typing counts: one glyph is one character, not the
    // two UTF-16 units it occupies.
    ["match", "🙂", false],
    ["match", "🙂🙂", true],
  ] as const)(
    "listsAnyone(%s intent, %o) is %s",
    (intent, q, expected) => {
      expect(listsAnyone(q, intent)).toBe(expected);
    },
  );

  // The account needle is ONE predicate, spaces included, so the field itself is
  // what the floor measures — `ad ex` matches a display name across the gap.
  // What a blank field means is the caller's: the accounts mode reviews the
  // whole fold, and inside one person that fold would bury the accounts they
  // actually hold.
  it.each([
    ["browse", "", true],
    ["browse", "   ", true],
    ["match", "", false],
    ["match", "   ", false],
    ["browse", "o", false],
    ["match", "o", false],
    ["browse", "oc", true],
    ["match", "oc", true],
    ["match", " oc ", true],
    ["match", "a b", true],
  ] as const)("listsAnyAccount(%s intent, %o) is %s", (intent, q, expected) => {
    expect(listsAnyAccount(q, intent)).toBe(expected);
  });
});

describe("useAccountList", () => {
  const searchAccounts = vi.mocked(identityClient.searchAccounts);

  function fold(): identityClient.AccountSearchResponse {
    return {
      items: [
        {
          source: "github",
          source_id: "01900000-0000-7000-8000-00000000aa01",
          account_id: "gh-1",
          email: "one@example.com",
          username: null,
          display_name: null,
          person: null,
          bound_by_operator: false,
        },
      ],
      next_cursor: null,
    };
  }

  // The same bug the person listing's intent key exists to prevent: `enabled:
  // false` stops a request and NOT a cache read, so a shared key would let the
  // in-person field render the whole fold the accounts mode had just browsed.
  it("does not serve the browsed fold to a match-intent caller", async () => {
    const { wrapper } = harness();
    searchAccounts.mockResolvedValue(fold());
    const browsed = renderHook(() => useAccountList("", "browse"), { wrapper });
    await waitFor(() => expect(browsed.result.current.isSuccess).toBe(true));

    const matching = renderHook(() => useAccountList("", "match"), { wrapper });

    expect(matching.result.current.data).toBeUndefined();
  });

  it.each([
    ["browse", true],
    ["match", false],
  ] as const)(
    "a blank field asks the service under %s intent: %s",
    async (intent, asks) => {
      const { wrapper } = harness();
      searchAccounts.mockResolvedValue(fold());

      const { result } = renderHook(() => useAccountList("", intent), { wrapper });

      if (asks) {
        await waitFor(() => expect(result.current.isSuccess).toBe(true));
      }
      expect(searchAccounts.mock.calls.length > 0).toBe(asks);
    },
  );
});

describe("usePersonList while a term is being typed", () => {
  function page(...names: string[]): identityClient.PersonSearchResponse {
    return {
      items: names.map((display_name, i) => ({
        person_id: `01900000-0000-7000-8000-0000000000${10 + i}`,
        display_name,
      })),
      next_cursor: null,
    };
  }

  // Every keystroke is its own query key. Without kept data the list empties to
  // a spinner between letters, which reads as "no matches" to the operator
  // mid-word.
  it("keeps the rows it already has while the next term loads", async () => {
    const { wrapper } = harness();
    searchPersons.mockResolvedValueOnce(page("Ada Example"));
    const { result, rerender } = renderHook(({ q }) => usePersonList(q, "match"), {
      wrapper,
      initialProps: { q: "ada" },
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    let resolveNext: (value: identityClient.PersonSearchResponse) => void = () => {};
    searchPersons.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveNext = resolve;
      }),
    );
    rerender({ q: "adam" });

    expect(result.current.data?.pages[0]?.items[0]?.display_name).toBe("Ada Example");
    expect(result.current.isFetching).toBe(true);

    resolveNext(page("Adam Other"));
    await waitFor(() =>
      expect(result.current.data?.pages[0]?.items[0]?.display_name).toBe("Adam Other"),
    );
  });

  // The service answers a dropped request all the same, so the cancellation has
  // to reach it: one journal scan per abandoned keystroke is the cost otherwise.
  it("passes an abort signal the query can cancel with", async () => {
    const { wrapper } = harness();
    searchPersons.mockResolvedValueOnce(page("Ada Example"));

    const { result } = renderHook(() => usePersonList("ada", "match"), { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(searchPersons.mock.calls[0][2]).toBeInstanceOf(AbortSignal);
  });
});

describe("the prune across cached limits", () => {
  // `keepPreviousData` leaves one cache entry per limit the operator reached.
  // A filter narrowed to the first page would leave the row they just decided
  // sitting in the longer list they are actually looking at.
  it("drops the decided row from every cached limit", async () => {
    const { queryClient, wrapper } = harness();
    const longer = ["identity", "resolution", "attention", "tenant-a", 400];
    for (const key of [ATTENTION_KEY, longer]) {
      queryClient.setQueryData(key, {
        items: [item("a1"), item("a2")],
        rates: { persons: 2, observed: 2, bound: 0, pending: 2, no_source_id: 0, no_evidence: 0, excluded: 0 },
      });
    }
    bindAccount.mockResolvedValue(outcome("applied"));

    const { result } = renderHook(() => useBindAccount(), { wrapper });
    result.current.mutate({
      account: { source: REF.source, source_id: REF.source_id, id: REF.account_id },
      person_id: "01900000-0000-7000-8000-0000000000b0",
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    for (const key of [ATTENTION_KEY, longer]) {
      const cached = queryClient.getQueryData(key) as { items: unknown[] };
      expect(cached.items, `under ${JSON.stringify(key)}`).toHaveLength(1);
    }
  });
});

describe("useAttention", () => {
  // The limit is the whole load-more feature. Keyed on it but not PASSED, every
  // press would fire a fresh request, blank nothing (`keepPreviousData`) and
  // answer the same 200 rows for ever — with every other test still green,
  // because they all mock this hook away.
  it("asks the service for the limit it is keyed on", async () => {
    vi.mocked(identityClient.getAttention).mockResolvedValue({
      items: [],
      rates: { observed: 0, bound: 0, pending: 0, no_source_id: 0, no_evidence: 0, excluded: 0 },
    });
    const { queryClient, wrapper } = harness();

    const { result } = renderHook(() => useAttention(400), { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(identityClient.getAttention).toHaveBeenCalledWith(400);
    expect(
      queryClient
        .getQueryCache()
        .find({ queryKey: ["identity", "resolution", "attention", "tenant-a", 400] }),
    ).toBeDefined();
  });

  // A longer read is a NEW key, and a new key is pending. Without the previous
  // answer standing in, the queue an operator is working blanks to a spinner
  // on every press.
  it("keeps the shorter answer on screen while the longer one is read", async () => {
    vi.mocked(identityClient.getAttention).mockResolvedValue({
      items: [],
      rates: { observed: 0, bound: 0, pending: 0, no_source_id: 0, no_evidence: 0, excluded: 0 },
    });
    const { queryClient, wrapper } = harness();
    const { result, rerender } = renderHook(({ limit }) => useAttention(limit), {
      wrapper,
      initialProps: { limit: 200 },
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    let resolve: (value: identityClient.AttentionResponse) => void = () => {};
    vi.mocked(identityClient.getAttention).mockReturnValueOnce(
      new Promise((r) => {
        resolve = r;
      }),
    );
    rerender({ limit: 400 });

    expect(result.current.isLoading).toBe(false);
    expect(result.current.data).toBeDefined();
    expect(result.current.isPlaceholderData).toBe(true);
    resolve({
      items: [],
      rates: { observed: 0, bound: 0, pending: 0, no_source_id: 0, no_evidence: 0, excluded: 0 },
    });
    await waitFor(() => expect(result.current.isPlaceholderData).toBe(false));
    queryClient.clear();
  });
});
