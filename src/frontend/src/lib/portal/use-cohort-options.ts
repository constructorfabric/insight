import { useMemo } from "react";

import { collectPeopleAttrs } from "@/lib/insight/slices";
import { normalizePersonId } from "@/lib/metrics/entity";
import {
  catalogAttributes,
  cohortOptions,
  type CohortOptions,
} from "@/lib/portal/cohort-options";
import { usePortalSlice } from "@/lib/portal/portal-nav";
import { useMetricDefinitionsResponse } from "@/queries/metric-definitions";
import { useVisibleRoster } from "@/queries/visible-roster";

export interface CohortOptionsState extends CohortOptions {
  /** True until both sources have answered — nothing may be dropped before. */
  isPending: boolean;
  /**
   * The stored slice, but only while it is one of `dims`.
   *
   * Two things read the slice: the control, which shows the selection, and the
   * cohort builder, which decides who a person is compared against. They used
   * to read it independently, so a stored value the options no longer contain
   * left the control showing "Team (all)" while the comparison was still built
   * by the old attribute — the screen said one thing and did another. Deriving
   * it once here removes the possibility rather than correcting it afterwards.
   */
  slice: string;
}

/**
 * The cohort choices, and which of them is in effect.
 *
 * One hook so the control and the comparison cannot disagree; see
 * `cohortOptions` for why the catalog outranks the roster.
 */
export function useCohortOptions(): CohortOptionsState {
  const stored = usePortalSlice();
  const definitions = useMetricDefinitionsResponse();
  const roster = useVisibleRoster(true);

  const options = useMemo(
    () =>
      cohortOptions(
        catalogAttributes(definitions.data),
        collectPeopleAttrs(roster.roster, normalizePersonId).values(),
      ),
    [definitions.data, roster.roster],
  );

  const isPending = definitions.isPending || roster.isPending;
  return {
    ...options,
    isPending,
    // While the sources are still answering, keep the stored value: a link
    // carrying `?slice=` must survive the moment before the options arrive.
    slice:
      isPending || options.dims.some((d) => d.key === stored) ? stored : "",
  };
}
