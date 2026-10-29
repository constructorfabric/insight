import { useMemo } from "react";

import { useViewer } from "@/auth";
import { DomainLensView, Pending } from "@/components/portal/domain-lens-view";
import { HomeDashboards } from "@/components/portal/home-dashboards";
import { MetricGroupsView } from "@/components/portal/metric-groups-view";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { visibleGroups } from "@/lib/insight/groups";
import { zoneById } from "@/lib/portal/nav-model";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useViewerReach } from "@/lib/portal/use-viewer-reach";
import { useZoneNav } from "@/lib/portal/use-zone-nav";
import { TEXT_TITLE } from "@/lib/type-scale";
import {
  DEFAULT_OVERVIEW_ITEM,
  OVERVIEW_ITEMS,
  overviewMetricKeys,
} from "@/lib/portal/overview-configs";

/**
 * Overview content — every pane item resolves through the overview-configs
 * registry and renders DomainLensView, exactly like Directions (Overview
 * design O2). The grid fetch is zone-scoped (union across all items) so
 * switching items never re-spins the loading gate.
 *
 * Its default item is Home, which every viewer lands on: their own glance and
 * dashboards, then the org rollup for a viewer who has a cohort to roll up.
 */
export function OverviewView({ item }: { item: string | null }) {
  const gridKeys = useMemo(() => overviewMetricKeys(), []);
  const { canSeeOthers, isPending } = useViewerReach();
  const asked = item ?? DEFAULT_OVERVIEW_ITEM;
  // An org theme waits for the viewer's reach: served early, an IC would see
  // a rollup that is then swapped for Home.
  if (isPending && asked !== DEFAULT_OVERVIEW_ITEM) {
    return <CenteredSpinner className="min-h-[60vh]" />;
  }
  const shown = canSeeOthers ? asked : DEFAULT_OVERVIEW_ITEM;
  const entry = OVERVIEW_ITEMS[shown];
  if (!entry) {
    return <Pending label={`“${item}” isn't an Overview view yet.`} />;
  }
  if (shown !== DEFAULT_OVERVIEW_ITEM) {
    return <DomainLensView config={entry} gridKeys={gridKeys} />;
  }

  return (
    <div className="@container flex flex-col gap-6 p-4 md:p-6">
      <h1 className={TEXT_TITLE}>Home</h1>
      <OwnGlance />
      <HomeDashboards />
      {canSeeOthers ? (
        <DomainLensView config={entry} gridKeys={gridKeys} embedded />
      ) : null}
    </div>
  );
}

function OwnGlance() {
  const { personId } = useViewer();
  const { selectZone } = useZoneNav();
  const showPlanned = usePortalShowPlanned();
  const groupIds = visibleGroups(showPlanned).map((group) => group.id);
  const personZone = zoneById("person");

  if (!personId) return null;

  return (
    <MetricGroupsView
      personId={personId}
      groupIds={groupIds}
      showKpis
      embedded
      onSelectGroup={(id) => {
        if (personZone) selectZone(personZone, id);
      }}
    />
  );
}
