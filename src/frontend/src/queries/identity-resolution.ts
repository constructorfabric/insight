/**
 * Data hooks for the identity-resolution operator console.
 *
 * All of these sit behind the admin gate server-side; components additionally
 * render them only inside `IdentitiesGate`, so a 403 here means the role was
 * revoked mid-session — surfaced as an error state, not silently retried.
 */
import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import { getAttention, type AttentionResponse } from "@/api/identity-client";
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
