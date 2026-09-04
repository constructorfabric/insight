import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  getGearBoards,
  getGearRoadmap,
  type GearBoard,
  type GearOrder,
  type GearRoadmap,
} from "@/api/gear-roadmap-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";

const KEY = ["gear-roadmap"] as const;
const BOARDS_KEY = ["gear-roadmap", "boards"] as const;

/** The board is synced on a schedule, so a minute-old answer is still current. */
const STALE_MS = 60_000;

/** Which boards exist changes only when a connector syncs a new one. */
const BOARDS_STALE_MS = 300_000;

export function useGearBoards(): UseQueryResult<GearBoard[]> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [...BOARDS_KEY, sessionAuthorizationScope(session)],
    queryFn: getGearBoards,
    staleTime: BOARDS_STALE_MS,
  });
}

export function useGearRoadmap(
  project: number | null,
  order: GearOrder | null,
): UseQueryResult<GearRoadmap> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [
      ...KEY,
      project,
      order?.sort ?? null,
      order?.direction ?? null,
      sessionAuthorizationScope(session),
    ],
    queryFn: () => getGearRoadmap(project as number, order),
    enabled: project !== null,
    staleTime: STALE_MS,
    // The rows are the same board in a different order, so keep the table on
    // screen while the next order arrives rather than blanking it.
    placeholderData: (previous) => previous,
  });
}
