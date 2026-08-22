import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getConnectorHealth,
  type ConnectorHealthResponse,
} from "@/api/connector-health-client";

export function useConnectorHealth(): UseQueryResult<ConnectorHealthResponse> {
  return useQuery({
    queryKey: ["connector-health"],
    queryFn: getConnectorHealth,
    staleTime: 5 * 60 * 1000,
  });
}
