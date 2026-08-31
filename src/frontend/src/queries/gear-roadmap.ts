import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import { getGearRoadmap, type GearRoadmap } from "@/api/gear-roadmap-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";

const KEY = ["gear-roadmap"] as const;

/** The board is synced on a schedule, so a minute-old answer is still current. */
const STALE_MS = 60_000;

export function useGearRoadmap(): UseQueryResult<GearRoadmap> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [...KEY, sessionAuthorizationScope(session)],
    queryFn: () => getGearRoadmap(),
    staleTime: STALE_MS,
  });
}
