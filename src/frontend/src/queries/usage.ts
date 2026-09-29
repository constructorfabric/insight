import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getUsageActions,
  getUsagePages,
  getUsagePeople,
  getUsageSummary,
  type UsageActionsSort,
  type UsageEvent,
  type UsageList,
  type UsageOrder,
  type UsagePage,
  type UsagePagesSort,
  type UsagePeopleSort,
  type UsagePerson,
  type UsageRange,
  type UsageSummary,
} from "@/api/usage-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import { keepWithinPeriod } from "@/queries/same-period";

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

function useUsageList<K extends string, T>(
  list: string,
  range: UsageRange,
  order: UsageOrder<K> | null,
  search: string,
  read: () => Promise<UsageList<T>>,
): UseQueryResult<UsageList<T>> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [
      "usage",
      list,
      sessionAuthorizationScope(session),
      range.since,
      range.until,
      order?.sort ?? null,
      order?.direction ?? null,
      search,
    ],
    queryFn: read,
    staleTime: 0,
    refetchOnMount: "always",
    placeholderData: keepWithinPeriod<UsageList<T>>(range),
  });
}

export function useUsagePeople(
  range: UsageRange,
  order: UsageOrder<UsagePeopleSort> | null,
  search = "",
): UseQueryResult<UsageList<UsagePerson>> {
  return useUsageList("people", range, order, search, () =>
    getUsagePeople(range, order, search),
  );
}

export function useUsagePages(
  range: UsageRange,
  order: UsageOrder<UsagePagesSort> | null,
): UseQueryResult<UsageList<UsagePage>> {
  return useUsageList("pages", range, order, "", () => getUsagePages(range, order));
}

export function useUsageActions(
  range: UsageRange,
  order: UsageOrder<UsageActionsSort> | null,
): UseQueryResult<UsageList<UsageEvent>> {
  return useUsageList("actions", range, order, "", () => getUsageActions(range, order));
}
