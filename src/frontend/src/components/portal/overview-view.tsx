import { useMemo } from "react";

import { DomainLensView, Pending } from "@/components/portal/domain-lens-view";
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
  return <DomainLensView config={entry} gridKeys={gridKeys} />;
}
