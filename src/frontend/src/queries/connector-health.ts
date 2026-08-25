import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getConnectorHealth,
  getConnectorRuns,
  type ConnectorHealthResponse,
  type ConnectorRunsResponse,
} from "@/api/connector-health-client";

/**
 * The facts are recorded on a controller cadence, so refetching faster than
 * that only re-reads the same rows. Short enough that a reopened page reflects
 * the last sweep, long enough that a tab left open is not a poll loop.
 */
const STALE_TIME_MS = 60 * 1000;

export function useConnectorHealth(): UseQueryResult<ConnectorHealthResponse> {
  return useQuery({
    queryKey: ["connector-health"],
    queryFn: getConnectorHealth,
    staleTime: STALE_TIME_MS,
  });
}

/** One connector's recent runs, fetched only when its row is expanded. */
export function useConnectorRuns(
  connector: string | null
): UseQueryResult<ConnectorRunsResponse> {
  return useQuery({
    queryKey: ["connector-health", "runs", connector],
    queryFn: () => getConnectorRuns(connector as string),
    enabled: connector !== null,
    staleTime: STALE_TIME_MS,
  });
}
