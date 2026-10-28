import type { PeopleListItem } from "@/api/identity-client";
import { useViewer } from "@/auth";
import { useVisibilityPolicy } from "@/queries/identity-me";
import { useVisibleRoster } from "@/queries/visible-roster";

/**
 * Whether the org zones roll up everyone the viewer may see rather than a
 * reporting subtree: under a flat policy, or for a viewer who manages no one
 * yet is shown other people.
 */
export function rollsUpWholeRoster(
  isFlat: boolean,
  roster: readonly PeopleListItem[],
  viewerPersonId: string | null | undefined
): boolean {
  if (isFlat) return true;
  if (!viewerPersonId) return false;

  const viewer = viewerPersonId.toLowerCase();
  const managesSomeone = roster.some(
    (person) => person.manager_person_id?.toLowerCase() === viewer
  );
  const seesSomeoneElse = roster.some(
    (person) => person.person_id.toLowerCase() !== viewer
  );

  return !managesSomeone && seesSomeoneElse;
}

export function useWholeRoster(): { wholeRoster: boolean; isPending: boolean } {
  const { personId } = useViewer();
  const { isFlat, isPending: policyPending } = useVisibilityPolicy();
  const visible = useVisibleRoster(true);

  return {
    wholeRoster: rollsUpWholeRoster(isFlat, visible.roster, personId),
    isPending: policyPending || visible.isPending,
  };
}
