import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getConnectorHealth,
  type ConnectorHealthResponse,
} from "@/api/connector-health-client";

/**
 * Short stale time: the page exists to answer "is data arriving right now", so
 * a long cache would let it report a stopped source as healthy.
 */
export function useConnectorHealth(): UseQueryResult<ConnectorHealthResponse> {
  return useQuery({
    queryKey: ["connector-health"],
    queryFn: getConnectorHealth,
    staleTime: 30 * 1000,
  });
}
