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
    // The app-wide default holds a query fresh for an hour, which suits tables
    // the pipeline rewrites daily. Usage is written as it happens, so stepping
    // back to a period already looked at must re-read rather than replay the
    // snapshot from then — otherwise a month reads smaller than the week in it.
    staleTime: 0,
    refetchOnMount: "always",
  });
}
