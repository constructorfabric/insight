import { useMemo } from "react";

import { useViewer } from "@/auth";
import { cohortKey, collectRosterAttrs } from "@/lib/insight/slices";
import { normalizePersonId } from "@/lib/metrics/entity";
import { useCohortOptions } from "@/lib/portal/use-cohort-options";
import { useIcPerson } from "@/queries/ic-dashboard";

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
  const { personId } = useViewer();
  const tree = useIcPerson(personId ?? "").data ?? null;
  const attrByEntity = useMemo(
    () => collectRosterAttrs(tree, normalizePersonId),
    [tree]
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
