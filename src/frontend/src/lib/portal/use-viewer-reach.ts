import { useViewerIsManager } from "@/lib/portal/use-viewer-is-manager";
import { useVisibilityPolicy } from "@/queries/identity-me";

/**
 * Whether the portal's org zones (Overview, People, Directions, AI & Cost, …)
 * have anyone to be about.
 *
 * They roll up a set of people, and there are two ways to have one: manage a
 * subtree, or belong to an organisation whose visibility policy is flat. A leaf
 * IC and a member of a hierarchy-less organisation are served the same empty
 * `subordinates`, so the tree alone cannot tell them apart — the policy does.
 *
 * `isManager` stays available for the surfaces that mean the reporting line
 * specifically, rather than "has people to look at".
 */
export interface ViewerReach {
  /** The org zones have a cohort — a subtree, or the whole organisation. */
  canSeeOthers: boolean;
  /** The viewer has direct or indirect reports. */
  isManager: boolean;
  /** Neither answer is in yet; callers should wait rather than collapse. */
  isPending: boolean;
}

export function useViewerReach(): ViewerReach {
  const { isManager, isPending: treePending } = useViewerIsManager();
  const policy = useVisibilityPolicy();

  return {
    canSeeOthers: policy.isFlat || isManager,
    isManager,
    isPending: treePending || policy.isPending,
  };
}
