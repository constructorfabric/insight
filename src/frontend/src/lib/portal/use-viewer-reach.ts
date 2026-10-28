import { useViewerIsManager } from "@/lib/portal/use-viewer-is-manager";
import { useWholeRoster } from "@/lib/portal/whole-roster";

/**
 * Whether the portal's org zones (Overview, People, Directions, AI & Cost, …)
 * have anyone to be about.
 *
 * They roll up a set of people: a managed subtree, or everyone the viewer may
 * see when they manage no one — under a flat policy, or when other people are
 * shown to them anyway. A leaf IC is shown only themselves.
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
  const { wholeRoster, isPending: wholePending } = useWholeRoster();

  return {
    canSeeOthers: wholeRoster || isManager,
    isManager,
    isPending: treePending || wholePending,
  };
}
