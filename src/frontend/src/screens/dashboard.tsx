import { useState } from "react";

import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { DashboardHeader } from "@/components/widgets/dashboard/dashboard-header";
import { IcNeedsAttention } from "@/components/widgets/dashboard/ic-needs-attention";
import {
  KpiTile,
  KpiTilePlaceholder,
} from "@/components/widgets/dashboard/kpi-tile";
import { MetricGroupCard } from "@/components/widgets/metric-views/metric-group-card";
import { GroupDrilldownSheet } from "@/components/widgets/dashboard/group-drilldown-sheet";
import { usePeriod } from "@/hooks/use-period";
import { useSettings } from "@/hooks/use-settings";
import { metricAttentionItems } from "@/lib/insight/attention";
import { metricKpiTiles, type KpiTileData } from "@/lib/insight/kpi-row";
import {
  GROUPS,
  KPI_ROW,
  KPI_ROW_COLLECTION,
  type GroupId,
} from "@/lib/insight/groups";
import {
  projectViews,
  type MetricCollectionConfig,
} from "@/lib/metrics/collection";
import { IdentityApiError } from "@/api/identity-client";
import { normalizePersonId } from "@/lib/metrics/entity";
import { useIcPerson } from "@/queries/ic-dashboard";
import {
  collectionSetPending,
  useMetricCollection,
  useMetricCollectionSet,
} from "@/queries/metric-results";

// Stable references so the disabled drilldown query keeps a constant key.
const EMPTY_COLLECTION: MetricCollectionConfig = { metrics: [] };
const CLOSED_ENTITY = { type: "person" as const, ids: [] };
// Placeholder for closed drilldown sheets; their body never renders.
const CLOSED_DRILLDOWN_DATA = {
  byKey: new Map(),
  previousByKey: null,
  isPending: true,
  isFetching: false,
  isError: false,
  refetch: () => {},
} as const;

export interface DashboardScreenProps {
  personId: string;
}

export function DashboardScreen({ personId }: DashboardScreenProps) {
  const personQ = useIcPerson(personId);
  const person = personQ.data ?? null;
  const { period, dateRange } = usePeriod();
  const { focusMode } = useSettings();
  const entityId = normalizePersonId(personId);
  const entity = { type: "person" as const, ids: [entityId] };

  const kpiData = useMetricCollection(KPI_ROW_COLLECTION, entity, dateRange, {
    previousPeriod: period,
  });
  // Cards only render period + peer; the heavy timeseries/breakdown views
  // exist for the drilldown. Fetch the light projection here so a card paints
  // as fast as a KPI tile, and let the open drilldown fetch the full
  // collection lazily below.
  const groupData = useMetricCollectionSet(
    GROUPS.map((def) => ({
      key: def.id,
      collection: projectViews(def.collection, ["period", "peer"]),
    })),
    entity,
    dateRange
  );

  const [openGroup, setOpenGroup] = useState<GroupId | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const openDetails = (group: GroupId) => {
    setOpenGroup(group);
    setDetailsOpen(true);
  };

  // Full collection for the open metrics group only (drives the drilldown's
  // chart blocks + peer story). Disabled while nothing is open — empty ids
  // gate the query off — so heavy views are never fetched for drilldowns the
  // user doesn't open.
  const openMetricDef =
    openGroup != null
      ? (GROUPS.find((def) => def.id === openGroup) ?? null)
      : null;
  const drilldownData = useMetricCollection(
    openMetricDef?.collection ?? EMPTY_COLLECTION,
    openMetricDef ? entity : CLOSED_ENTITY,
    dateRange
  );

  // Never fall back to the raw id (a UUID) — a person outside the viewer's
  // cached tree resolves in a beat; the title stays blank until then.
  const displayName = person?.display_name ?? "";
  const role = person?.job_title;

  // The one loading gate: a single page spinner while any of the screen's
  // queries has no data. A period change mints new query keys, so the same
  // gate re-trips — no per-widget loaders, no partial paints.
  const isLoading = kpiData.isPending || collectionSetPending(groupData);
  // Identity failing is not a metric failure: with no person there is no name,
  // no reports, and the metrics below are unauthorized anyway. A 404 means the
  // id is gone or outside the viewer's visible set — say so, rather than paint
  // a nameless dashboard over requests that all fail.
  const personMissing =
    personQ.error instanceof IdentityApiError && personQ.error.status === 404;

  const tiles = metricKpiTiles(
    kpiData.byKey,
    kpiData.previousByKey,
    entityId,
    focusMode
  );
  const tilesByKey = new Map<string, KpiTileData>(
    tiles.map((tile) => [tile.key, tile])
  );

  const attentionItems = GROUPS.flatMap((def) =>
    metricAttentionItems(
      def,
      groupData.get(def.id)?.byKey ?? new Map(),
      entityId
    )
  );

  // Close any open drilldown when the viewed person changes. Render-phase
  // reset against the previous id rather than an effect (no cascading commit).
  const [prevPersonId, setPrevPersonId] = useState(personId);
  if (personId !== prevPersonId) {
    setPrevPersonId(personId);
    setOpenGroup(null);
    setDetailsOpen(false);
  }

  return (
    <div className="flex flex-col">
      <DashboardHeader
        title={displayName}
        subtitle={role}
        person={personId}
        hasReports={(person?.subordinates?.length ?? 0) > 0}
      />
      <main className="flex flex-1 flex-col gap-8 p-4 md:p-6">
        {personQ.isError ? (
          <ComingSoon
            variant="card"
            state="error"
            label={
              personMissing
                ? "This person is not available"
                : "Unable to load this person"
            }
            onRetry={personMissing ? undefined : () => void personQ.refetch()}
          />
        ) : isLoading ? (
          <CenteredSpinner className="min-h-[70vh]" />
        ) : (
          <>
            <section className="flex flex-col gap-3">
              <p className="flex items-center gap-1.5 text-xs font-medium tracking-wider text-muted-foreground uppercase">
                At a glance
              </p>
              <div className="grid grid-cols-[repeat(auto-fit,minmax(13rem,1fr))] gap-3">
                {KPI_ROW.map((key) => {
                  const tile = tilesByKey.get(key);
                  if (tile) {
                    return (
                      <KpiTile
                        key={key}
                        tile={tile}
                        onOpenGroup={openDetails}
                      />
                    );
                  }
                  if (kpiData.isError) {
                    return (
                      <ComingSoon
                        key={key}
                        variant="card"
                        state="error"
                        onRetry={kpiData.refetch}
                      />
                    );
                  }
                  return <KpiTilePlaceholder key={key} />;
                })}
              </div>
            </section>

            <IcNeedsAttention
              items={attentionItems}
              onOpenGroup={openDetails}
            />

            <section className="flex flex-col gap-3">
              <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
                Sections
              </p>
              <div className="grid grid-cols-[repeat(auto-fit,minmax(18rem,1fr))] gap-3">
                {GROUPS.map((def) => {
                  const result = groupData.get(def.id);
                  if (!result) return null;
                  return (
                    <MetricGroupCard
                      key={def.id}
                      def={def}
                      data={result}
                      entityId={entityId}
                      onOpen={() => openDetails(def.id)}
                    />
                  );
                })}
              </div>
            </section>
          </>
        )}
      </main>

      {GROUPS.map((def) => (
        <GroupDrilldownSheet
          key={def.id}
          open={detailsOpen && openGroup === def.id}
          onOpenChange={setDetailsOpen}
          onOpenChangeComplete={(open) => {
            if (!open && openGroup === def.id) setOpenGroup(null);
          }}
          def={def}
          metricTarget={{
            kind: "person",
            entityId,
            // The drilldown for the open group reads the full-collection
            // query; closed sheets never render their body.
            data: def.id === openGroup ? drilldownData : CLOSED_DRILLDOWN_DATA,
          }}
          range={dateRange}
          period={period}
          cohortLabel="department"
        />
      ))}
    </div>
  );
}
