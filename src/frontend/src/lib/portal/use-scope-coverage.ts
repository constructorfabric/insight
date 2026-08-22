import { useMemo } from "react";

import {
  coverageDistribution,
  partCoverage,
  personCoverage,
  reachableMetricKeys,
  thinlyCovered,
  type CoverageDistribution,
  type PartCoverage,
  type PersonCoverage,
} from "@/lib/insight/coverage";
import { visibleGroups } from "@/lib/insight/groups";
import { projectViews } from "@/lib/metrics/collection";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useMetricDefinitionsResponse } from "@/queries/metric-definitions";
import { useMetricCollectionSet } from "@/queries/metric-results";

export interface ScopeCoverage {
  distribution: CoverageDistribution;
  /** Per person, so a level can be opened into the people at it. */
  people: readonly PersonCoverage[];
  /** People seen in fewer than half their parts — the screen's finding. */
  thin: number;
  /**
   * A request failed, so no claim may be made. Not a detail: with the
   * definition listing unavailable nothing is known to reach the tenant, and
   * every part would read "no data reaches us" for every person — an
   * infrastructure fault rendered as a confident verdict about named people,
   * which is the failure the three states exist to prevent. The caller must
   * check this before reading anything else here.
   */
  isError: boolean;
  /** The same coverage cut by part, from the same states. */
  parts: readonly PartCoverage[];
  isPending: boolean;
}

const CLOSED = { type: "person" as const, ids: [] as string[] };

/**
 * How much of their work the product can see, for everyone the viewer may see.
 *
 * Computed in the browser, over the viewer's visible set. Both are compromises
 * and both are stated: `distribution.counted` says how many people the answer
 * covers, and nothing here is presented as being about the organisation unless
 * the viewer's reach is the organisation.
 *
 * That compromise is affordable here and is NOT affordable for a statistic. A
 * quartile over a subset is a different quantity from the same quartile over
 * the whole group, and biased. A count over a subset stays a true statement
 * about that subset as long as the subset's size travels with it — which is
 * why `counted` is not optional and not cosmetic.
 *
 * The roster is the id list, exactly. The visibility check on the metrics
 * endpoint is all-or-nothing: one id outside the caller's visible set refuses
 * the whole request rather than filtering it, and does not say which id was at
 * fault. So the list is built from the tree the viewer was served and is never
 * widened or guessed.
 */
export function useScopeCoverage(
  memberIds: readonly string[],
): ScopeCoverage {
  const { dateRange } = usePortalPeriod();
  // Sections this install does not show are not gaps in anyone's data, so they
  // are not counted — see `visibleGroups`.
  const showPlanned = usePortalShowPlanned();
  const groups = useMemo(() => visibleGroups(showPlanned), [showPlanned]);
  // The scope selector owns who is in view, so this takes the member list
  // rather than deriving one. Deriving it would leave the tab answering about
  // the viewer's whole reach while every other tab answered about the selected
  // scope, and the two would silently disagree on the same screen.
  const rosterIds = useMemo(() => [...memberIds], [memberIds]);

  // `period` only, deliberately. A collection carrying timeseries, breakdown
  // or histogram views cannot be chunked (`entityChunkSize` returns null for
  // them), and an unchunked roster-sized request runs into the backend's
  // projected-row limit. Asking for the one view this needs keeps the existing
  // chunk-and-merge path available at roster scale.
  const data = useMetricCollectionSet(
    rosterIds.length
      ? groups.map((def) => ({
          key: def.id,
          collection: projectViews(def.collection, ["period"]),
        }))
      : [],
    rosterIds.length ? { type: "person" as const, ids: rosterIds } : CLOSED,
    dateRange,
  );

  // The same query key the availability gate uses, so this rides its cache
  // rather than issuing a second listing request.
  const definitions = useMetricDefinitionsResponse();
  const reachable = useMemo(
    () => reachableMetricKeys(definitions.data?.metrics ?? []),
    [definitions.data],
  );

  return useMemo(() => {
    const byKey = new Map(
      groups.flatMap((def) => [...(data.get(def.id)?.byKey ?? new Map())]),
    );
    const people = rosterIds.map((id) =>
      personCoverage(groups, byKey, id, reachable),
    );
    return {
      distribution: coverageDistribution(people, groups.length),
      people,
      thin: thinlyCovered(people, groups.length),
      parts: partCoverage(groups, people),
      // An empty roster is an answer, not a wait. With no members the hook
      // sends no collections, so no group has an entry and the `?? true`
      // below would hold every one of them pending forever — the section
      // would sit on its loading label for good rather than saying there is
      // nobody in this scope.
      isPending:
        definitions.isPending ||
        (rosterIds.length > 0 &&
          groups.some((def) => data.get(def.id)?.isPending ?? true)),
      isError:
        definitions.isError ||
        groups.some((def) => data.get(def.id)?.isError ?? false),
    };
  }, [
    data,
    groups,
    rosterIds,
    reachable,
    definitions.isPending,
    definitions.isError,
  ]);
}
