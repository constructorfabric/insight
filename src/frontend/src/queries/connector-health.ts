import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getConnectorHealth,
  getConnectorSyncs,
  type ConnectorHealthSummary,
  type ConnectorInstanceRef,
  type ConnectorSyncHistory,
} from "@/api/connector-health-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import { instanceKey } from "@/lib/portal/connector-health";

const KEY = ["connector-health"] as const;

/** How often the open page asks again. Well inside the reconcile cadence, so
 * the age it prints is never more than a minute behind the truth. */
const RECHECK_MS = 60_000;

export function useConnectorHealth(): UseQueryResult<ConnectorHealthSummary> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [...KEY, "summary", sessionAuthorizationScope(session)],
    queryFn: () => getConnectorHealth(),
    // The answer is only as fresh as the last sweep, and an operator opening
    // this page during an incident wants what is recorded now — not what was
    // recorded when they last looked.
    staleTime: 0,
    refetchOnMount: "always",
    // INVARIANT: the page's own freshness line is the gap between two stamps
    // the SERVER sent, so it does not move on its own. Without this the age
    // freezes the moment the page renders, and an operator watching an incident
    // would never see recording stop.
    refetchInterval: RECHECK_MS,
  });
}

/**
 * One installation's history, keyed by the whole identity.
 *
 * Keying on the connector name alone would serve one installation's window
 * from the other's cache entry, because two installations share the name.
 */
export function useConnectorSyncs(
  instance: ConnectorInstanceRef | null,
): UseQueryResult<ConnectorSyncHistory> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [
      ...KEY,
      "syncs",
      instance === null ? null : instanceKey(instance),
      sessionAuthorizationScope(session),
    ],
    queryFn: () => getConnectorSyncs(instance as ConnectorInstanceRef),
    enabled: instance !== null,
    staleTime: 0,
  });
}
