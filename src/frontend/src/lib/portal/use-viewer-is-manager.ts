import { useViewer } from "@/auth";
import { useVisibleRoster } from "@/queries/visible-roster";

/**
 * Whether the current viewer manages anyone — the reporting line itself; who
 * the org zones roll up is `useViewerReach`'s answer.
 *
 * `isPending` remains true until the canonical roster resolves; callers should
 * treat pending as "assume manager" to avoid hiding zones on a flash.
 */
export function useViewerIsManager(): {
  isManager: boolean;
  isPending: boolean;
  isError: boolean;
} {
  const { personId } = useViewer();
  const roster = useVisibleRoster(true);
  const viewerId = personId?.toLowerCase();
  const isManager = viewerId
    ? roster.roster.some(
        (person) => person.manager_person_id?.toLowerCase() === viewerId,
      )
    : false;

  return {
    isManager,
    isPending: roster.isPending || roster.isError,
    isError: roster.isError,
  };
}
