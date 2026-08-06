import { useState } from "react";

import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { GroupDrilldownSheet } from "@/components/widgets/dashboard/group-drilldown-sheet";
import { IcNeedsAttention } from "@/components/widgets/dashboard/ic-needs-attention";
import { KpiTile, KpiTilePlaceholder } from "@/components/widgets/dashboard/kpi-tile";
import { MetricGroupCard } from "@/components/widgets/metric-views/metric-group-card";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { useSettings } from "@/hooks/use-settings";
import { metricAttentionItems } from "@/lib/insight/attention";
import {
  KPI_ROW,
  KPI_ROW_COLLECTION,
  GROUPS,
  type GroupId,
} from "@/lib/insight/groups";
import { metricKpiTiles, type KpiTileData } from "@/lib/insight/kpi-row";
import { injectCohortPeer } from "@/lib/insight/within-team-peer";
import {
  projectViews,
  type MetricCollectionConfig,
} from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import { useCohortLabel } from "@/lib/portal/use-cohort-label";
import { usePersonCohort } from "@/lib/portal/use-person-cohort";
import {
  collectionSetPending,
  useMetricCollection,
  useMetricCollectionSet,
} from "@/queries/metric-results";

const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };
const CLOSED_ENTITY = { type: "person" as const, ids: [] };
const CLOSED_DRILLDOWN_DATA = {
  byKey: new Map(),
  previousByKey: null,
  isPending: true,
  isFetching: false,
  isError: false,
  refetch: () => {},
} as const;

/** The person and catalogue slice a metric-groups screen renders. */
export interface MetricGroupsViewProps {
  /** Person id the lens is scoped to (org-level rollup is a backend follow-up). */
  personId: string;
  /** Which metric-family groups to render; empty ⇒ nothing available yet. */
  groupIds: readonly GroupId[];
  /** Overview renders the KPI row + needs-attention; direction lenses don't. */
  showKpis?: boolean;
  /**
   * When set, a group card selects the group via this callback (e.g. to expand
   * it inline in a second sidebar level) instead of opening the drilldown
   * modal. Passing it also suppresses the modal sheets entirely.
   */
  onSelectGroup?: (id: GroupId) => void;
  /**
   * Render the per-group section cards. Off ⇒ a general glance (KPI row +
   * needs-attention only) with sections reached from the sidebar instead.
   */
  showSections?: boolean;
}

/**
 * Catalog-driven metric-group screen — the reusable body behind the portal's
 * Overview and Direction lenses. Reuses the v2 dashboard widgets
 * (KpiTile / MetricGroupCard / GroupDrilldownSheet) and the `/metric-results`
 * collection queries, parameterised by the set of groups to show.
 */
export function MetricGroupsView({
  personId,
  groupIds,
  showKpis = false,
  onSelectGroup,
  showSections = true,
}: MetricGroupsViewProps) {
  const cohortLabel = useCohortLabel();
  const { period, dateRange } = usePortalPeriod();
  const { focusMode } = useSettings();
  const entityId = normalizePersonId(personId);
  const entity = { type: "person" as const, ids: [entityId] };

  const defs = GROUPS.filter((d) => groupIds.includes(d.id));

  const kpiData = useMetricCollection(
    showKpis ? KPI_ROW_COLLECTION : EMPTY_COLLECTION,
    showKpis ? entity : CLOSED_ENTITY,
    dateRange,
    { previousPeriod: period },
  );
  const groupData = useMetricCollectionSet(
    defs.map((def) => ({
      key: def.id,
      collection: projectViews(def.collection, ["period", "peer"]),
    })),
    entity,
    dateRange,
  );

  // Slice cohort: the people who share this person's active-slice attribute
  // value. Only fetched when a slice is picked — otherwise the person's own
  // numbers stand alone (no cohort, tiles show "no peer data" as before).
  const cohortIds = usePersonCohort(entityId);
  const cohortEntity =
    cohortIds.length ? { type: "person" as const, ids: cohortIds } : CLOSED_ENTITY;
  const cohortKpi = useMetricCollection(
    cohortIds.length && showKpis ? KPI_ROW_COLLECTION : EMPTY_COLLECTION,
    // Entity gated on the SAME condition as the collection — a live entity
    // with an empty collection still issues a useless network request.
    cohortIds.length && showKpis ? cohortEntity : CLOSED_ENTITY,
    dateRange,
  );
  const cohortGroup = useMetricCollectionSet(
    cohortIds.length
      ? defs.map((def) => ({
          key: def.id,
          collection: projectViews(def.collection, ["period"]),
        }))
      : [],
    cohortEntity,
    dateRange,
  );

  const [openGroup, setOpenGroup] = useState<GroupId | null>(null);
  // With `onSelectGroup` a card/alert navigates to the section inline; without
  // it, it opens the drilldown modal.
  const openOrSelect = onSelectGroup ?? setOpenGroup;
  const openDef =
    openGroup != null ? (defs.find((d) => d.id === openGroup) ?? null) : null;
  const drilldownData = useMetricCollection(
    openDef?.collection ?? EMPTY_COLLECTION,
    openDef ? entity : CLOSED_ENTITY,
    dateRange,
  );

  const [prevPersonId, setPrevPersonId] = useState(personId);
  if (personId !== prevPersonId) {
    setPrevPersonId(personId);
    setOpenGroup(null);
  }

  if (defs.length === 0) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="empty"
          label="Not in the semantic layer yet — bullet-only direction"
        />
      </div>
    );
  }

  const isLoading = (showKpis && kpiData.isPending) || collectionSetPending(groupData);
  if (isLoading) return <CenteredSpinner className="min-h-[60vh]" />;

  // Surface a backend failure as a retryable error, not empty section cards.
  const isError =
    (showKpis && kpiData.isError) || [...groupData.values()].some((r) => r.isError);
  if (isError)
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          onRetry={() => {
            kpiData.refetch();
            groupData.forEach((r) => r.refetch());
          }}
        />
      </div>
    );

  // Cohort-injected views: with a slice active, the person's results carry
  // their cohort's peer stats so tiles/cards/attention read "vs <slice> median".
  const kpiByKey = injectCohortPeer(kpiData.byKey, cohortKpi.byKey, cohortIds);
  const groupResult = (id: GroupId) => {
    const r = groupData.get(id);
    if (!r) return undefined;
    const cr = cohortGroup.get(id);
    if (!cohortIds.length || !cr) return r;
    return { ...r, byKey: injectCohortPeer(r.byKey, cr.byKey, cohortIds) };
  };

  const tiles = showKpis
    ? metricKpiTiles(kpiByKey, kpiData.previousByKey, entityId, focusMode)
    : [];
  const tilesByKey = new Map<string, KpiTileData>(
    tiles.map((tile) => [tile.key, tile]),
  );
  const attentionItems = showKpis
    ? defs.flatMap((def) =>
        metricAttentionItems(def, groupResult(def.id)?.byKey ?? new Map(), entityId),
      )
    : [];

  return (
    <>
      <main className="flex flex-1 flex-col gap-8 p-4 md:p-6">
        {showKpis ? (
          <>
            <section className="flex flex-col gap-3">
              <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
                At a glance
              </p>
              <div className="grid grid-cols-[repeat(auto-fit,minmax(13rem,1fr))] gap-3">
                {/* KPI_ROW is a plain metric-key list upstream now — the
                    legacy/metric tile split died with the legacy data path. */}
                {KPI_ROW.map((key) => {
                  const tile = tilesByKey.get(key);
                  if (tile)
                    return (
                      <KpiTile key={key} tile={tile} onOpenGroup={openOrSelect} />
                    );
                  return <KpiTilePlaceholder key={key} />;
                })}
              </div>
            </section>
            <IcNeedsAttention items={attentionItems} onOpenGroup={openOrSelect} />
          </>
        ) : null}

        {showSections ? (
          <section className="flex flex-col gap-3">
            <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
              Sections
            </p>
            <div className="grid grid-cols-[repeat(auto-fit,minmax(18rem,1fr))] gap-3">
              {defs.map((def) => {
                const result = groupResult(def.id);
                if (!result) return null;
                return (
                  <MetricGroupCard
                    key={def.id}
                    def={def}
                    data={result}
                    entityId={entityId}
                    onOpen={() => openOrSelect(def.id)}
                  />
                );
              })}
            </div>
          </section>
        ) : null}
      </main>

      {onSelectGroup || !showSections
        ? null
        : defs.map((def) => (
            <GroupDrilldownSheet
              key={def.id}
              open={openGroup === def.id}
              onOpenChange={(o) => setOpenGroup(o ? def.id : null)}
              def={def}
              metricTarget={{
                kind: "person",
                entityId,
                data:
                  def.id === openGroup ? drilldownData : CLOSED_DRILLDOWN_DATA,
              }}
              range={dateRange}
              period={period}
              cohortLabel={cohortLabel}
            />
          ))}
    </>
  );
}
