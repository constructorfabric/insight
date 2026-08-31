/**
 * Preview-experiment hooks and the capability gate that decides whether the
 * `/previews` surface exists for this viewer at all.
 *
 * The gate is two-layered and entirely session-derived (insight#2374):
 * `experiments_enabled` is the stand-level switch `/auth/me` echoes, and the
 * session `roles` are the identity role names the authenticator minted into
 * the gateway JWT. Both are the same values the previews service enforces
 * server-side — the UI check is a courtesy, never the boundary (the same
 * doctrine as `queries/identity-me.ts`).
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

/**
 * The role names that may manage previews — mirrored by the previews
 * service's scope gate and seeded by identity migrations 007 / 017. Names,
 * not ids: the JWT `roles` claim (and therefore the session) carries names.
 */
export const PREVIEWS_MANAGE_ROLES = ["previews-admin", "admin"] as const;

/** Cluster state changes under other operators' hands; half a minute is fine. */
const EXPERIMENTS_STALE_TIME = 30 * 1000;

/**
 * Whether this viewer gets the previews surface: the stand capability is on
 * AND the session carries a managing role. Pending/absent sessions read as
 * "no" — the shell simply never draws the entry.
 */
export function canManagePreviews(session: Session | null): boolean {
  if (!session?.experimentsEnabled) return false;
  return session.roles.some((role) =>
    (PREVIEWS_MANAGE_ROLES as readonly string[]).includes(role),
  );
}

/** The session-derived previews gate, as a hook for the nav and the screen. */
export function usePreviewsGate(): boolean {
  const { session } = useAuth();
  return canManagePreviews(session);
}

export function useExperiments(): UseQueryResult<Experiment[]> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  return useQuery({
    // Keyed by the session scope so a sign-out/sign-in (or view-as) never
    // serves the previous caller's listing from cache.
    queryKey: ["previews", "experiments", sessionScope],
    queryFn: listExperiments,
    staleTime: EXPERIMENTS_STALE_TIME,
    enabled: sessionScope != null,
  });
}

/** Everything previews reads lives under this prefix — one invalidation after
 *  any mutation refreshes the listing. */
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
