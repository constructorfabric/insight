import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getUsageSummary,
  type UsageRange,
  type UsageSummary,
} from "@/api/usage-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";

export function useUsageSummary(range: UsageRange): UseQueryResult<UsageSummary> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [
      "usage",
      "summary",
      sessionAuthorizationScope(session),
      range.since,
      range.until,
    ],
    queryFn: () => getUsageSummary(range),
    staleTime: 0,
    refetchOnMount: "always",
  });
}
