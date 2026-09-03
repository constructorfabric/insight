import { useViewer } from "@/auth";
import { useVisibleRoster } from "@/queries/visible-roster";

/**
 * Whether the current viewer manages anyone. The portal's org zones (Overview,
 * People, Directions, AI & Cost, …) all roll up the viewer's subtree, so they
 * only mean something for a manager. An individual contributor has no subtree,
 * so those zones would be empty — the shell collapses to their Person page
 * instead (see LensRail / PortalLayout).
 *
 * `isPending` remains true until the canonical roster resolves; callers should
 * treat pending as "assume manager" to avoid hiding zones on a flash.
 */
export function useViewerIsManager(): { isManager: boolean; isPending: boolean } {
  const { personId } = useViewer();
  const roster = useVisibleRoster(true);
  const viewerId = personId?.toLowerCase();
  const isManager = viewerId
    ? roster.roster.some(
        (person) => person.manager_person_id?.toLowerCase() === viewerId,
      )
    : false;

  return { isManager, isPending: roster.isPending || roster.isError };
}
