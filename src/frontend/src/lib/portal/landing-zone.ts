import {
  ZONES_FOR_EVERYONE,
  zoneById,
  zoneIsPlanned,
} from "@/lib/portal/nav-model";
import {
  navPolicy,
  zoneHidden,
  type InstanceNavPolicy,
} from "@/lib/portal/nav-policy";

/**
 * The landing-zone decision: what the portal shell does with the zone in the
 * URL once the viewer's shape (manager? admin?) resolves.
 *
 * Home (where the install shows it), the viewer's own page and Dashboards are
 * everyone's, so a bare /portal and those stand without waiting. An org zone
 * waits for the viewer's reach, and Manage for the admin role: an admin
 * operator may manage no one.
 */
export type LandingDecision =
  /** An input is still resolving — decide on a later render. */
  | { kind: "wait" }
  /** The zone in the URL stands. */
  | { kind: "keep" }
  /** The zone is not this viewer's to see — back to Home. */
  | { kind: "reset" };

export function landingDecision(args: {
  zone: string | null;
  mgrPending: boolean;
  canSeeOthers: boolean;
  adminPending: boolean;
  isAdmin: boolean;
  homeShown: boolean;
}): LandingDecision {
  const { zone, mgrPending, canSeeOthers, adminPending, isAdmin, homeShown } =
    args;

  if (zone === "overview" && !homeShown) return { kind: "reset" };
  if (zone == null || ZONES_FOR_EVERYONE.has(zone)) return { kind: "keep" };
  if (mgrPending) return { kind: "wait" };
  if (canSeeOthers) return { kind: "keep" };

  // Manage is gated by the admin role, not by having reports. Hold the
  // decision until the role answer is in: resetting first would discard the
  // URL the admin deliberately opened.
  if (zone === "manage") {
    if (adminPending) return { kind: "wait" };
    return isAdmin ? { kind: "keep" } : { kind: "reset" };
  }
  return { kind: "reset" };
}

/** The zone a bare /portal opens: Home, unless the install leaves it out. */
export function bareLandingZone(
  showPlanned: boolean,
  policy: InstanceNavPolicy = navPolicy()
): "overview" | "person" {
  const home = zoneById("overview");
  const homeShown =
    home !== undefined &&
    !zoneHidden(home.id, policy) &&
    (!zoneIsPlanned(home, policy) || showPlanned);
  return homeShown ? "overview" : "person";
}
