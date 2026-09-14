import { useMemo } from "react";

import { cohortKey, collectPeopleAttrs } from "@/lib/insight/slices";
import { normalizePersonId } from "@/lib/metrics/entity";
import { useCohortOptions } from "@/lib/portal/use-cohort-options";
import { useVisibleRoster } from "@/queries/visible-roster";

/**
 * The entity ids of a person's slice cohort — everyone in the viewer's org who
 * shares that person's value for the active slice attribute. Empty when no
 * slice is active (so callers skip the cohort fetch and show the person's own
 * numbers alone). Shared by every person-scoped view so slice support is wired
 * once, not re-derived per screen.
 */
export function usePersonCohort(entityId: string): string[] {
  // The slice the OPTIONS still contain, not whatever is stored: a value the
  // catalog has since dropped would otherwise keep building cohorts while the
  // control shows "Team (all)".
  const { slice } = useCohortOptions();
  const roster = useVisibleRoster(true).roster;
  const attrByEntity = useMemo(
    () => collectPeopleAttrs(roster, normalizePersonId),
    [roster],
  );
  return useMemo(() => {
    if (!slice) return [];
    // Membership via the shared `cohortKey` predicate so this hook can never
    // drift from how every other surface derives cohorts.
    const own = cohortKey(attrByEntity.get(entityId), slice);
    if (own == null) return [];
    return [...attrByEntity.entries()]
      .filter(([, a]) => cohortKey(a, slice) === own)
      .map(([id]) => id);
  }, [attrByEntity, slice, entityId]);
}
