/**
 * Data hooks for the identity-resolution operator console.
 *
 * All of these sit behind the admin gate server-side; components additionally
 * render them only inside `IdentitiesGate`, so a 403 here means the role was
 * revoked mid-session — surfaced as an error state, not silently retried.
 */
import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseMutationResult,
  type UseQueryResult,
} from "@tanstack/react-query";

import {
  bindAccount,
  detachAccount,
  excludeAccount,
  getAccountBinding,
  getAttention,
  getPersonAccounts,
  mergePersons,
  searchPersons,
  type AccountBinding,
  type AttentionResponse,
  type CorrectionResponse,
  type PersonAccountEntry,
  type PersonSearchResponse,
} from "@/api/identity-client";
import type { AccountRef } from "@/lib/identities/account-key";
import { useAuth } from "@/auth/use-auth";
import { sessionAuthorizationScope } from "@/auth/session-scope";

/** An operator works a queue; a minute of staleness is fine, losing edits is not. */
const ATTENTION_STALE_TIME = 60 * 1000;

export function useAttention(): UseQueryResult<AttentionResponse> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  return useQuery({
    queryKey: ["identity", "resolution", "attention", sessionScope],
    queryFn: () => getAttention(),
    staleTime: ATTENTION_STALE_TIME,
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
    // Invalidate-only, the house style: the journal is the truth and a verb
    // may land as `already_decided`/`refused`, so guessing the new state
    // client-side would lie exactly when it matters.
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: RESOLUTION_KEY });
      void client.invalidateQueries({ queryKey: PERSON_SEARCH_KEY });
    },
  });
}

export const useBindAccount = () => useCorrection(bindAccount);
export const useMergePersons = () => useCorrection(mergePersons);
export const useDetachAccount = () => useCorrection(detachAccount);
export const useExcludeAccount = () => useCorrection(excludeAccount);

/** Live person search for the picker; the component debounces the input. */
export function usePersonSearch(q: string): UseQueryResult<PersonSearchResponse> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const trimmed = q.trim();
  return useQuery({
    queryKey: ["identity", "persons", "search", sessionScope, trimmed],
    queryFn: () => searchPersons(trimmed),
    staleTime: ATTENTION_STALE_TIME,
    enabled: sessionScope != null && trimmed.length >= 2,
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
