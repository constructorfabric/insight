import { useMemo } from "react";

import { useViewer } from "@/auth";
import { availableSlices, collectRosterAttrs } from "@/lib/insight/slices";
import { normalizePersonId } from "@/lib/metrics/entity";
import { usePortalSlice } from "@/lib/portal/portal-nav";
import { useIcPerson } from "@/queries/ic-dashboard";
import { useVisibilityPolicy } from "@/queries/identity-me";

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
  isFlat: boolean
): string {
  if (sliceLabel) return midSentence(sliceLabel);
  return isFlat ? "organisation" : "team";
}

export function useCohortLabel(): string {
  const slice = usePortalSlice();
  const { isFlat } = useVisibilityPolicy();
  const { personId } = useViewer();
  const tree = useIcPerson(personId ?? "").data ?? null;
  const dims = useMemo(
    () => availableSlices(collectRosterAttrs(tree, normalizePersonId).values()),
    [tree]
  );
  const sliceLabel = slice
    ? (dims.find((dimension) => dimension.key === slice)?.label ?? "cohort")
    : null;

  return peerPopulationLabel(sliceLabel, isFlat);
}
