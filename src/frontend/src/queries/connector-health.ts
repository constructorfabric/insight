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
