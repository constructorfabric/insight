import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getUsageSummary,
  type UsageRange,
  type UsageSummary,
} from "@/api/usage-client";

export function useUsageSummary(
  range: UsageRange,
  enabled: boolean,
): UseQueryResult<UsageSummary> {
  return useQuery({
    queryKey: ["usage", "summary", range.since ?? "", range.until ?? ""],
    queryFn: () => getUsageSummary(range),
    enabled,
  });
}
