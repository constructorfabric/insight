/**
 * Data hooks for the identity-resolution operator console.
 *
 * All of these sit behind the admin gate server-side; components additionally
 * render them only inside `IdentitiesGate`, so a 403 here means the role was
 * revoked mid-session — surfaced as an error state, not silently retried.
 */
import {
  keepPreviousData,
  useInfiniteQuery,
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
  type InfiniteData,
  type UseInfiniteQueryResult,
  type UseMutationResult,
  type UseQueryResult,
} from "@tanstack/react-query";

import {
  bindAccount,
  bindAccounts,
  detachAccount,
  excludeAccount,
  getAccountBinding,
  getAttention,
  getPersonAccounts,
  mergePersons,
  QUEUE_FIRST_PAGE,
  searchAccounts,
  searchPersons,
  type AccountBinding,
  type AccountSearchResponse,
  type AttentionResponse,
  type CorrectionResponse,
  type PersonAccountEntry,
  type PersonSearchResponse,
} from "@/api/identity-client";
import type { AccountRef } from "@/lib/identities/account-key";
import { dropDecided } from "@/lib/identities/cases";
import { useAuth } from "@/auth/use-auth";
import { sessionAuthorizationScope } from "@/auth/session-scope";

/** An operator works a queue; a minute of staleness is fine, losing edits is not. */
const ATTENTION_STALE_TIME = 60 * 1000;

/**
 * The review queue, capped at `limit` items.
 *
 * Raising the limit is what "load more" means here — the service derives the
 * queue from the whole tenant on every read, so there is no cursor to resume
 * from, and a bigger ask returns a longer prefix of the same order.
 */
export function useAttention(
  limit: number = QUEUE_FIRST_PAGE,
): UseQueryResult<AttentionResponse> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  return useQuery({
    queryKey: ["identity", "resolution", "attention", sessionScope, limit],
    queryFn: () => getAttention(limit),
    staleTime: ATTENTION_STALE_TIME,
    // A longer ask re-reads the rows already on screen: without this the queue
    // an operator is working blanks to a spinner every time they ask for more.
    placeholderData: keepPreviousData,
    enabled: sessionScope != null,
  });
}

export function useAccountBinding(
  ref: AccountRef | null,
): UseQueryResult<AccountBinding> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  return useQuery({
    queryKey: [
      "identity",
      "resolution",
      "account",
      sessionScope,
      ref?.source,
      ref?.source_id,
      ref?.account_id,
    ],
    queryFn: () => {
      if (!ref) throw new Error("account ref is missing");
      return getAccountBinding(ref);
    },
    staleTime: ATTENTION_STALE_TIME,
    enabled: sessionScope != null && ref != null,
  });
}

/** Everything the console reads lives under this prefix — one invalidation
 *  after any verb refreshes the queue, the rates and the open account. */
const RESOLUTION_KEY = ["identity", "resolution"] as const;

/** The picker's search cache lives outside {@link RESOLUTION_KEY} but a verb
 *  changes its answers too: a merge absorbs a person the cache would keep
 *  offering, and binding to them recreates the split the operator just fixed. */
const PERSON_SEARCH_KEY = ["identity", "persons", "search"] as const;

type Verb<TArgs> = UseMutationResult<CorrectionResponse, unknown, TArgs>;

function useCorrection<TArgs>(
  run: (args: TArgs) => Promise<CorrectionResponse>,
): Verb<TArgs> {
  const client = useQueryClient();
  return useMutation({
    mutationFn: run,
    // The journal stays the truth, and the refetch below is what reconciles
    // to it — but the attention read folds every observed account, so its
    // latency grows with the dataset, and until it lands the operator is
    // still looking at the row they just decided. Dropping the accounts the
    // SERVER reported as decided is not a guess about the new state; a
    // `refused` account keeps its row, and the rest follows from the refetch.
    onSuccess: (result) => {
      client.setQueriesData<AttentionResponse>(
        { queryKey: [...RESOLUTION_KEY, "attention"] },
        (previous) =>
          previous
            ? { ...previous, items: dropDecided(previous.items, result.items) }
            : previous,
      );
      void client.invalidateQueries({ queryKey: RESOLUTION_KEY });
      void client.invalidateQueries({ queryKey: PERSON_SEARCH_KEY });
    },
  });
}

export const useBindAccount = () => useCorrection(bindAccount);
export const useBindAccounts = () => useCorrection(bindAccounts);
export const useMergePersons = () => useCorrection(mergePersons);
export const useDetachAccount = () => useCorrection(detachAccount);
export const useExcludeAccount = () => useCorrection(excludeAccount);

/** Rows per page. Enough to fill the panel, small enough to stay one screen. */
const PAGE_SIZE = 50;

/**
 * The person listing, a page at a time. A blank query lists the whole tenant —
 * the terms narrow the same list, which is why they share one hook rather than
 * a separate "browse" one.
 *
 * The query is part of the key, so narrowing the terms starts a new walk: the
 * service refuses a cursor issued for a different query, since resuming one
 * mid-alphabet would skip people.
 */
/** What a blank query means to a caller of {@link usePersonList}. */
export type PersonListIntent = "browse" | "match";

/**
 * The same for {@link useAccountList}. The accounts mode browses — reviewing
 * what the connectors reported is the point there. Inside one person a blank
 * field asks nothing: the tenant's whole fold would bury the handful of accounts
 * that person actually holds, which is what the reader opened them for.
 */
export type AccountListIntent = "browse" | "match";

/**
 * How much has to be typed before a term searches at all.
 *
 * A single character names most of the roster, so the answer is no use to the
 * operator — and the service still pays a pass over the person journal to
 * produce it. Two is where a term starts to mean something: it is also the whole
 * of a name in scripts that write one glyph per syllable, which a higher floor
 * would put out of reach entirely. A blank field is a different question and
 * keeps its own answer.
 */
export const MIN_SEARCH_CHARS = 2;

/**
 * How long a field waits before it searches. Past the gap between two
 * keystrokes of ordinary typing (150-300 ms), so a typed word costs one search
 * of the journal rather than one per letter.
 */
export const SEARCH_DEBOUNCE_MS = 400;

/**
 * Characters as the person typing counts them. `String.length` counts UTF-16
 * units, so one astral glyph reads as two and would buy a search on its own.
 */
function typedLength(q: string): number {
  return [...q].length;
}

/**
 * The shortest predicate the person search would actually send.
 *
 * INVARIANT: measured per TERM, not per field. The service splits `q` on
 * whitespace and matches every term against the journal on its own, so `a b` is
 * two one-character passes — the field length would call that three characters
 * and wave through exactly the query the floor exists to stop.
 */
function shortestTerm(q: string): number {
  const terms = q.split(/\s+/).filter((term) => term.length > 0);
  return terms.length === 0 ? 0 : Math.min(...terms.map(typedLength));
}

/**
 * The account needle is ONE predicate, its spaces included — `ada ex` matches a
 * display name across the gap — so there the field's own length is the measure.
 */
function needleLength(q: string): number {
  return typedLength(q.trim());
}

/**
 * Whether these terms ask for anything under this intent.
 *
 * INVARIANT: one rule, used by both the query's fetch gate and the caller's
 * display. A picker that renders rows the query never asked for is showing
 * another caller's cache — and one that renders rows for terms too short to
 * search is showing the answer to a different question.
 */
export function listsAnyone(q: string, intent: PersonListIntent): boolean {
  return searches(shortestTerm(q), intent === "browse");
}

/**
 * The same question for the account listing. Its needle is one predicate, so the
 * floor measures the field; what a blank field means is the caller's to say.
 */
export function listsAnyAccount(q: string, intent: AccountListIntent): boolean {
  return searches(needleLength(q), intent === "browse");
}

/**
 * Typed something, but not yet enough to search — what the person field says
 * instead of going silent. The message names the term, because that is what the
 * floor measures.
 */
export function belowPersonFloor(q: string): boolean {
  return belowFloor(shortestTerm(q));
}

/** The same for the account field, over its single needle. */
export function belowAccountFloor(q: string): boolean {
  return belowFloor(needleLength(q));
}

/** The floor itself: blank asks for a listing only where one is wanted. */
function searches(measured: number, blankLists: boolean): boolean {
  if (measured === 0) return blankLists;
  return measured >= MIN_SEARCH_CHARS;
}

function belowFloor(measured: number): boolean {
  return measured > 0 && measured < MIN_SEARCH_CHARS;
}

export function usePersonList(
  q: string,
  /** `browse` lists the tenant on a blank query; `match` answers nothing until
   *  terms are typed — the assign picker inside a dialog wants matches, and a
   *  roster would bury the one name the operator came to type.
   *
   *  INVARIANT: part of the query key, not only the fetch gate. `enabled: false`
   *  stops a request and not a cache read, so one shared key would render the
   *  roster that browse mode cached inside the dialog. */
  intent: PersonListIntent = "browse",
): UseInfiniteQueryResult<InfiniteData<PersonSearchResponse>> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const trimmed = q.trim();
  return useInfiniteQuery({
    queryKey: ["identity", "persons", "search", sessionScope, intent, trimmed],
    // The signal is the point, not hygiene: a search-as-you-type field
    // supersedes its own request, and each one costs the service a scan of the
    // person journal. Dropped on the client, they would all still be answered.
    queryFn: ({ pageParam, signal }) =>
      searchPersons(trimmed, { cursor: pageParam, limit: PAGE_SIZE }, signal),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    staleTime: ATTENTION_STALE_TIME,
    // One more keystroke is a new key: without this the list empties to a
    // spinner between every pair of letters.
    placeholderData: keepPreviousData,
    enabled: sessionScope != null && listsAnyone(trimmed, intent),
  });
}

/** The observed accounts, a page at a time; a blank query lists them all. */
export function useAccountList(
  q: string,
  /** `browse` lists every open account on a blank query; `match` lists none. */
  intent: AccountListIntent = "browse",
): UseInfiniteQueryResult<InfiniteData<AccountSearchResponse>> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const trimmed = q.trim();
  return useInfiniteQuery({
    // The intent is part of the key for the same reason it is on the person
    // listing: `enabled: false` stops a request and NOT a cache read, so a
    // shared key would let the in-person field render the whole fold the
    // accounts mode had just browsed.
    queryKey: [...RESOLUTION_KEY, "account-search", sessionScope, intent, trimmed],
    queryFn: ({ pageParam, signal }) =>
      searchAccounts(trimmed, { cursor: pageParam, limit: PAGE_SIZE }, signal),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    staleTime: ATTENTION_STALE_TIME,
    placeholderData: keepPreviousData,
    enabled: sessionScope != null && listsAnyAccount(trimmed, intent),
  });
}

/** The accounts a merge would move — fetched only while the preview is open. */
export function usePersonAccounts(
  personId: string | null,
): UseQueryResult<{ person_id: string; accounts: PersonAccountEntry[] }> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  return useQuery({
    queryKey: [...RESOLUTION_KEY, "person-accounts", sessionScope, personId],
    queryFn: () => {
      if (!personId) throw new Error("person id is missing");
      return getPersonAccounts(personId);
    },
    staleTime: ATTENTION_STALE_TIME,
    enabled: sessionScope != null && personId != null,
  });
}

/** What a merge of MANY persons into one would move, per absorbed person. */
export interface AccountsToMove {
  /** Every read landed. The preview is the consent, so a partial count must
   *  never be presented as the list of what moves. */
  ready: boolean;
  failed: boolean;
  accounts: PersonAccountEntry[];
  refetch: () => void;
}

/**
 * The accounts a case-level merge would move: one read per absorbed person,
 * flattened.
 *
 * A case can hold more than two people, and the merge endpoint takes exactly
 * two — so the survivor absorbs the rest one call at a time, and the preview
 * has to cover all of them before the operator consents to any.
 */
export function usePersonAccountsMany(personIds: string[]): AccountsToMove {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  return useQueries({
    queries: personIds.map((personId) => ({
      queryKey: [...RESOLUTION_KEY, "person-accounts", sessionScope, personId],
      queryFn: () => getPersonAccounts(personId),
      staleTime: ATTENTION_STALE_TIME,
      enabled: sessionScope != null,
    })),
    combine: (results) => ({
      // No reads to make is not "not landed yet": conflating them would leave a
      // caller with an empty list waiting on a spinner that never resolves.
      ready: results.every((r) => r.data != null),
      failed: results.some((r) => r.isError),
      accounts: results.flatMap((r) => r.data?.accounts ?? []),
      refetch: () => {
        for (const result of results) void result.refetch();
      },
    }),
  });
}
