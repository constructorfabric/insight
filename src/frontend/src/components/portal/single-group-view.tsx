import { useMemo } from "react";

import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { CollectionDrilldown } from "@/components/widgets/metric-views/collection-drilldown";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { GROUPS, type GroupId } from "@/lib/insight/groups";
import { injectCohortPeer } from "@/lib/insight/within-team-peer";
import { type MetricCollectionConfig } from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import { useCohortLabel } from "@/lib/portal/use-cohort-label";
import { usePersonCohort } from "@/lib/portal/use-person-cohort";
import { useMetricCollection } from "@/queries/metric-results";

const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };
const CLOSED_ENTITY = { type: "person" as const, ids: [] };

/**
 * A direction lens with a single metric group renders the group's full
 * drilldown inline — no click-to-expand card, since there's only one section.
 * Fetches the full collection (all views) so charts + peer story paint directly.
 * When a slice is active, the person's slice cohort is fetched and its peer
 * stats are injected so the drilldown's peer story reads "vs <slice> median".
 */
export function SingleGroupView({
  personId,
  groupId,
}: {
  personId: string;
  groupId: GroupId;
}) {
  const cohortLabel = useCohortLabel();
  const { dateRange } = usePortalPeriod();
  const entityId = normalizePersonId(personId);
  const def = GROUPS.find((d) => d.id === groupId) ?? null;

  const data = useMetricCollection(
    def?.collection ?? EMPTY_COLLECTION,
    def ? { type: "person", ids: [entityId] } : CLOSED_ENTITY,
    dateRange,
  );
  const cohortIds = usePersonCohort(entityId);
  const cohortData = useMetricCollection(
    def && cohortIds.length ? def.collection : EMPTY_COLLECTION,
    cohortIds.length ? { type: "person", ids: cohortIds } : CLOSED_ENTITY,
    dateRange,
  );
  const injectedData = useMemo(
    () => ({
      ...data,
      byKey: injectCohortPeer(data.byKey, cohortData.byKey, cohortIds),
    }),
    [data, cohortData.byKey, cohortIds],
  );

  if (!def) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="empty" label="Unknown group" />
      </div>
    );
  }
  if (data.isPending) return <CenteredSpinner className="min-h-[60vh]" />;
  // A failed fetch must surface as a retryable error, not a drilldown
  // rendered over an empty dataset (same policy as MetricGroupsView).
  if (data.isError) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="error" onRetry={() => data.refetch()} />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <h1 className="text-xl font-semibold tracking-tight">{def.title}</h1>
      <CollectionDrilldown
        def={def}
        data={injectedData}
        entityId={entityId}
        range={dateRange}
        cohortLabel={cohortLabel}
      />
    </div>
  );
}
