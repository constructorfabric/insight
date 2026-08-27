import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getConnectorHealth,
  getConnectorSyncs,
  type ConnectorHealthSummary,
  type ConnectorSyncHistory,
} from "@/api/connector-health-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";

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

export function useConnectorSyncs(
  connector: string | null,
): UseQueryResult<ConnectorSyncHistory> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [...KEY, "syncs", connector, sessionAuthorizationScope(session)],
    queryFn: () => getConnectorSyncs(connector as string),
    enabled: connector !== null,
    staleTime: 0,
  });
}
