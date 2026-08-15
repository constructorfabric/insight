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
    // Keyed by the session scope so a sign-out/sign-in (or view-as) never
    // serves the previous caller's usage from cache.
    queryKey: [
      "usage",
      "summary",
      sessionAuthorizationScope(session),
      range.since,
      range.until,
    ],
    queryFn: () => getUsageSummary(range),
  });
}
