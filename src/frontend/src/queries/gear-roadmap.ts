import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getGearRoadmap,
  type GearOrder,
  type GearRoadmap,
} from "@/api/gear-roadmap-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";

const KEY = ["gear-roadmap"] as const;

/** The board is synced on a schedule, so a minute-old answer is still current. */
const STALE_MS = 60_000;

export function useGearRoadmap(order: GearOrder): UseQueryResult<GearRoadmap> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [
      ...KEY,
      order.sort,
      order.direction,
      sessionAuthorizationScope(session),
    ],
    queryFn: () => getGearRoadmap(order),
    staleTime: STALE_MS,
    // The rows are the same board in a different order, so keep the table on
    // screen while the next order arrives rather than blanking it.
    placeholderData: (previous) => previous,
  });
}
