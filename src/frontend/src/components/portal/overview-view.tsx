import { useMemo } from "react";

import { DomainLensView, Pending } from "@/components/portal/domain-lens-view";
import { HomeDashboards } from "@/components/portal/home-dashboards";
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
 */
export function OverviewView({ item }: { item: string | null }) {
  const gridKeys = useMemo(() => overviewMetricKeys(), []);
  const entry = OVERVIEW_ITEMS[item ?? DEFAULT_OVERVIEW_ITEM];
  if (!entry) {
    return <Pending label={`“${item}” isn't an Overview view yet.`} />;
  }
  if ((item ?? DEFAULT_OVERVIEW_ITEM) === DEFAULT_OVERVIEW_ITEM) {
    return (
      <div className="@container flex flex-col gap-6 p-4 md:p-6">
        <h1 className={TEXT_TITLE}>Home</h1>
        <HomeDashboards />
        <DomainLensView config={entry} gridKeys={gridKeys} embedded />
      </div>
    );
  }
  return <DomainLensView config={entry} gridKeys={gridKeys} />;
}
