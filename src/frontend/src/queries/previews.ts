/**
 * Preview-experiment hooks and the session-derived gate for the `/previews`
 * surface: the stand's `experiments_enabled` AND a managing session role.
 * The previews service enforces the same roles — the UI check is a courtesy,
 * never the boundary (same doctrine as `queries/identity-me.ts`).
 */
import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseMutationResult,
  type UseQueryResult,
} from "@tanstack/react-query";

import {
  createExperiment,
  deleteExperiment,
  listExperiments,
  type CreateExperimentRequest,
  type Experiment,
} from "@/api/previews-client";
import { useAuth } from "@/auth/use-auth";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import type { Session } from "@/auth/types";

/** Mirrors the previews service's scope gate; the session carries NAMES. */
export const PREVIEWS_MANAGE_ROLES = ["previews-admin", "admin"] as const;

/** Cluster state changes under other operators' hands; half a minute is fine. */
const EXPERIMENTS_STALE_TIME = 30 * 1000;

/** Both layers at once; an absent session reads as "no" (fail closed). */
export function canManagePreviews(session: Session | null): boolean {
  if (!session?.experimentsEnabled) return false;
  return session.roles.some((role) =>
    (PREVIEWS_MANAGE_ROLES as readonly string[]).includes(role),
  );
}

export function usePreviewsGate(): boolean {
  const { session } = useAuth();
  return canManagePreviews(session);
}

export function useExperiments(): UseQueryResult<Experiment[]> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  return useQuery({
    // Session-scoped key: a sign-in never serves another caller's cache.
    queryKey: ["previews", "experiments", sessionScope],
    queryFn: listExperiments,
    staleTime: EXPERIMENTS_STALE_TIME,
    enabled: sessionScope != null,
  });
}

/** One invalidation after any mutation refreshes everything previews reads. */
const PREVIEWS_KEY = ["previews"] as const;

export function useCreateExperiment(): UseMutationResult<
  Experiment,
  unknown,
  CreateExperimentRequest
> {
  const client = useQueryClient();
  return useMutation({
    mutationFn: createExperiment,
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: PREVIEWS_KEY });
    },
  });
}

export function useDeleteExperiment(): UseMutationResult<
  void,
  unknown,
  string
> {
  const client = useQueryClient();
  return useMutation({
    mutationFn: deleteExperiment,
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: PREVIEWS_KEY });
    },
  });
}
