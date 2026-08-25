import { visibleGroups, type GroupId } from "@/lib/insight/groups";
import { groupHasData } from "@/lib/insight/group-data";
import { partState, reachableMetricKeys } from "@/lib/insight/coverage";
import { injectCohortPeer } from "@/lib/insight/within-team-peer";
import { countableSignals } from "@/lib/insight/metric-containment";
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
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useMetricDefinitionsResponse } from "@/queries/metric-definitions";
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
  /**
   * Whether anything feeding this section reaches the tenant. With `hasData`
   * false it separates a section this person is absent from (it reaches us,
   * they did none of it) from one nobody is measured in (nothing feeds it).
   *
   * Read from the tenant-wide definition listing, NOT from whether the
   * comparison pool happens to hold readings. The pool is whoever the viewer
   * can see, so a small pool with no user of a live system would report that
   * system as missing — and the smaller the viewer's reach, the more often. It
   * also made this page and the org-wide coverage screen answer the same
   * question two ways, and they were free to disagree.
   */
  peersHaveData: boolean;
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
  const showPlanned = usePortalShowPlanned();
  const groups = visibleGroups(showPlanned);

  const groupData = useMetricCollectionSet(
    groups.map((def) => ({
      key: def.id,
      collection: projectViews(def.collection, ["period", "peer"]),
    })),
    entity,
    dateRange,
  );

  // Same query key as the availability gate, so this rides its cache.
  const definitions = useMetricDefinitionsResponse();

  // The same cohort the screens compare against, so the nav mark and the
  // section it points at cannot disagree.
  const cohortIds = usePersonCohort(entityId);
  const cohortGroup = useMetricCollectionSet(
    cohortIds.length
      ? groups.map((def) => ({
          key: def.id,
          collection: projectViews(def.collection, ["period"]),
        }))
      : [],
    cohortIds.length ? { type: "person" as const, ids: cohortIds } : CLOSED_ENTITY,
    dateRange,
  );

  const reachable = reachableMetricKeys(definitions.data?.metrics ?? []);

  return groups.map((def) => {
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
    const counts = rankCounts(
      countableSignals(
        ranks,
        (entry) => entry.row,
        (entry) => entry.rank,
      ),
    );

    return {
      id: def.id,
      title: def.title,
      status: gradeSectionStanding(counts),
      phrase: sectionStandingPhrase(counts),
      hasData: groupHasData(def, byKey, entityId),
      peersHaveData:
        partState(def, byKey, entityId, reachable) !== "no_data_reaches_us",
      // The listing counts too. Without it `reachable` is empty, so every
      // section reads as one nothing reaches — the page would tell a reader we
      // see nothing about this person, then flip a moment later. The old pool
      // inference read the same query whose pending state was already tracked,
      // so this gap arrived with the new source.
      isPending: definitions.isPending || (result?.isPending ?? true),
    };
  });
}
