/**
 * The landing-zone decision: what the portal shell does with the zone in the
 * URL once the viewer's shape (manager? admin?) resolves.
 *
 * Pulled out of the layout effect because the rules braid three async facts
 * together and the mistakes are silent: the original effect reset EVERY
 * non-person zone for a non-manager, which kicked an admin operator (an IC by
 * design — the seeded operator sits outside the org chart) off
 * `?zone=manage` before the admin check could even answer.
 */
export type LandingDecision =
  /** An input is still resolving — decide on a later render. */
  | { kind: "wait" }
  /** The zone in the URL stands. */
  | { kind: "keep" }
  /** A viewer with a cohort landing on bare /portal starts at the org rollup. */
  | { kind: "pin-overview" }
  /** The zone is not this viewer's to see — back to route-driven (Person). */
  | { kind: "reset" };

export function landingDecision(args: {
  zone: string | null;
  mgrPending: boolean;
  canSeeOthers: boolean;
  adminPending: boolean;
  isAdmin: boolean;
  overviewVisible?: boolean;
}): LandingDecision {
  const {
    zone,
    mgrPending,
    canSeeOthers,
    adminPending,
    isAdmin,
    overviewVisible = true,
  } = args;

  if (mgrPending) return { kind: "wait" };
  if (canSeeOthers) {
    if (zone != null) return { kind: "keep" };
    return overviewVisible ? { kind: "pin-overview" } : { kind: "keep" };
  }

  // A viewer with nobody to look at collapses to Person — except Manage, which is
  // gated by the admin role, not by having reports. Hold the decision until
  // the role answer is in: resetting first would discard the URL the admin
  // deliberately opened.
  if (zone == null || zone === "person") return { kind: "keep" };
  if (zone === "manage") {
    if (adminPending) return { kind: "wait" };
    return isAdmin ? { kind: "keep" } : { kind: "reset" };
  }
  return { kind: "reset" };
}
