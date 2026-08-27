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

import {
  getMe,
  type MeResponse,
  type VisibilityPolicy,
} from "@/api/identity-client";
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

/** How often an ERRORED role check re-asks on its own — see `useMe`. */
const ME_ERROR_RETRY_MS = 30 * 1000;

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
    // The app-wide client never refetches on focus/reconnect (an analytics
    // cadence), and the portal shell's observers of this query never remount
    // — an errored check (the app opened during an identity blip) would
    // otherwise read "not an admin" for the rest of the SPA session, with no
    // affordance anywhere to recover. An authorization gate earns its own
    // recovery: refetch on focus and reconnect, and poll slowly while
    // errored until an answer lands.
    refetchOnWindowFocus: true,
    refetchOnReconnect: true,
    refetchInterval: (query) =>
      query.state.status === "error" ? ME_ERROR_RETRY_MS : false,
  });
}

export interface AdminGate {
  /** False until proven admin — the pending state renders as "not admin". */
  isAdmin: boolean;
  /** True while the answer is not in yet; nav can avoid flashing either way. */
  isPending: boolean;
  /** The check itself FAILED — "could not verify", which is not the same
   *  answer as "not an admin". Gates still fail closed on it; surfaces that
   *  refuse should say so and offer {@link AdminGate.retry} instead of
   *  telling a real admin to go ask for a role they hold. */
  isError: boolean;
  /** Re-ask now — the retry affordance for the error state. */
  retry: () => void;
}

/** Whether the viewer holds the active `admin` identity role. Fails closed. */
export function useIsAdmin(): AdminGate {
  const me = useMe();
  return {
    isAdmin:
      me.data?.roles.some((role) => role.role_id === ADMIN_ROLE_ID) ?? false,
    isPending: me.isPending,
    isError: me.isError,
    retry: () => void me.refetch(),
  };
}

export interface VisibilityPolicyState {
  policy: VisibilityPolicy;
  /** The organisation has no reporting lines to derive sight from. */
  isFlat: boolean;
  isPending: boolean;
}

/**
 * Which visibility rule this deployment runs.
 *
 * A leaf IC and a member of a hierarchy-less organisation are both served an
 * empty `subordinates`, so the shell cannot tell them apart by looking at
 * the tree — it asks here instead.
 *
 * Unknown reads as `org_chart`: pending, errored, and an older service that
 * omits the field all keep the rail as narrow as it is today. Widening it on a
 * failed check would open org zones to a viewer whose reach nobody confirmed.
 */
export function useVisibilityPolicy(): VisibilityPolicyState {
  const me = useMe();
  const policy: VisibilityPolicy = me.data?.visibility_policy ?? "org_chart";
  return { policy, isFlat: policy === "flat", isPending: me.isPending };
}
