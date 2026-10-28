import { useMemo } from "react";

import { availableSlices, collectPeopleAttrs } from "@/lib/insight/slices";
import { normalizePersonId } from "@/lib/metrics/entity";
import { usePortalSlice } from "@/lib/portal/portal-nav";
import { useWholeRoster } from "@/lib/portal/whole-roster";
import { useVisibleRoster } from "@/queries/visible-roster";

/**
 * The label reads mid-sentence ("above the division median"), so a plain
 * capitalised word is lowered. Anything else is left as authored: identity
 * attribute labels are becoming generic (constructorfabric/insight#1881), and
 * lowering them blindly turns "R&D area" into "r&d area".
 */
function midSentence(label: string): string {
  return /^[A-Z][a-z]*$/.test(label) ? label.toLowerCase() : label;
}

export function peerPopulationLabel(
  sliceLabel: string | null,
  wholeRoster: boolean
): string {
  if (sliceLabel) return midSentence(sliceLabel);
  return wholeRoster ? "organisation" : "team";
}

export function useCohortLabel(): string {
  const slice = usePortalSlice();
  const { wholeRoster } = useWholeRoster();
  const roster = useVisibleRoster(true).roster;
  const dims = useMemo(
    () => availableSlices(collectPeopleAttrs(roster, normalizePersonId).values()),
    [roster],
  );
  const sliceLabel = slice
    ? (dims.find((dimension) => dimension.key === slice)?.label ?? "cohort")
    : null;

  return peerPopulationLabel(sliceLabel, wholeRoster);
}
