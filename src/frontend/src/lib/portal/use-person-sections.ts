import { GROUPS, type GroupId } from "@/lib/insight/groups";
import { groupHasData } from "@/lib/insight/group-data";
import { injectCohortPeer } from "@/lib/insight/within-team-peer";
import {
  gradeSectionStanding,
  rankCounts,
  sectionStandingPhrase,
} from "@/lib/scoring";
import { forEntity, projectViews } from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import { derivePeerStanding } from "@/lib/metrics/peer-standing";
import { usePersonCohort } from "@/lib/portal/use-person-cohort";
import type { Status } from "@/lib/status";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { useMetricCollectionSet } from "@/queries/metric-results";

export interface SectionStanding {
  id: GroupId;
  title: string;
  /** Colour of the section's mark; neutral when nothing is rankable. */
  status: Status;
  /** "5 of 12 behind peers" — the count states its own scope. */
  phrase: string;
  /** False when no metric of the section has a value for this period. */
  hasData: boolean;
  isPending: boolean;
}

const CLOSED_ENTITY = { type: "person" as const, ids: [] as string[] };

/**
 * Where a person stands in each section, for the nav.
 *
 * The person page is an OVERVIEW and every section has its own screen, so the
 * page's job is to say which section is worth opening — and the place a reader
 * opens one is the nav. Cards restating three metrics per section answered a
 * different question and duplicated the list of sections that was already on
 * screen to their left.
 *
 * The queries here are the same ones the section screens run, so react-query
 * serves them from cache: the mark costs no extra request.
 */
export function usePersonSectionStandings(personId: string): SectionStanding[] {
  const { dateRange } = usePortalPeriod();
  const entityId = normalizePersonId(personId);
  const entity = { type: "person" as const, ids: [entityId] };

  const groupData = useMetricCollectionSet(
    GROUPS.map((def) => ({
      key: def.id,
      collection: projectViews(def.collection, ["period", "peer"]),
    })),
    entity,
    dateRange,
  );

  // The same cohort the screens compare against, so the nav mark and the
  // section it points at cannot disagree.
  const cohortIds = usePersonCohort(entityId);
  const cohortGroup = useMetricCollectionSet(
    cohortIds.length
      ? GROUPS.map((def) => ({
          key: def.id,
          collection: projectViews(def.collection, ["period"]),
        }))
      : [],
    cohortIds.length ? { type: "person" as const, ids: cohortIds } : CLOSED_ENTITY,
    dateRange,
  );

  return GROUPS.map((def) => {
    const result = groupData.get(def.id);
    const cohort = cohortGroup.get(def.id);
    const byKey =
      result && cohortIds.length && cohort
        ? injectCohortPeer(result.byKey, cohort.byKey, cohortIds)
        : (result?.byKey ?? new Map());

    const ranks = def.collection.metrics.flatMap((m) => {
      const metric = byKey.get(m.key);
      if (!metric) return [];
      const data = forEntity(metric, entityId);
      const standing = derivePeerStanding(metric.direction, {
        value: data.value,
        peer:
          metric.peer?.values.find(
            (v: { entity_id: string }) => v.entity_id === entityId,
          ) ?? null,
      });
      return [{ row: m.key, rank: standing.rank }];
    });
    const counts = rankCounts(ranks);

    return {
      id: def.id,
      title: def.title,
      status: gradeSectionStanding(counts),
      phrase: sectionStandingPhrase(counts),
      hasData: groupHasData(def, byKey, entityId),
      isPending: result?.isPending ?? true,
    };
  });
}
