/**
 * The viewer's identity roles (`GET /api/identity/v1/me`) and the admin gate
 * derived from them.
 *
 * Admin-ness is an active row in the identity service's `person_roles` table,
 * checked server-side on every admin request — it is NOT the `insight-admin`
 * realm role the session's `roles` array carries, and no identity endpoint
 * reads that array. This hook is therefore the only honest source for "show
 * the admin surfaces?": gating on the session roles would draw an admin
 * console the backend then refuses with 403.
 *
 * The UI check is a courtesy, not a boundary — every admin endpoint re-runs
 * its own gate regardless of what the frontend renders.
 */
import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import { getMe, type MeResponse } from "@/api/identity-client";
import { useAuth } from "@/auth/use-auth";
import { sessionAuthorizationScope } from "@/auth/session-scope";

/**
 * The seeded `admin` role id — a stable migration constant, mirrored by the
 * backend's `roles_repo::ADMIN_ROLE_ID`. Gating on the id rather than the
 * name survives a rename of the display label.
 */
export const ADMIN_ROLE_ID = "a4d11000-0000-4000-8000-000000000001";

/** Grants change rarely; a minute keeps a granted operator from waiting long. */
const ME_STALE_TIME = 60 * 1000;

export function useMe(): UseQueryResult<MeResponse> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  return useQuery({
    // Keyed by the session scope so a sign-out/sign-in (or view-as) never
    // serves the previous caller's roles from cache.
    queryKey: ["identity", "me", sessionScope],
    queryFn: getMe,
    staleTime: ME_STALE_TIME,
    enabled: sessionScope != null,
  });
}

export interface AdminGate {
  /** False until proven admin — the pending state renders as "not admin". */
  isAdmin: boolean;
  /** True while the answer is not in yet; nav can avoid flashing either way. */
  isPending: boolean;
}

/** Whether the viewer holds the active `admin` identity role. Fails closed. */
export function useIsAdmin(): AdminGate {
  const me = useMe();
  return {
    isAdmin:
      me.data?.roles.some((role) => role.role_id === ADMIN_ROLE_ID) ?? false,
    isPending: me.isPending,
  };
}
