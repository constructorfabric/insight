import { useMemo } from "react";

import { useViewer } from "@/auth";
import { ScopeSelect } from "@/components/portal/scope-select";
import { SliceSelect } from "@/components/portal/slice-select";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { PeriodSelectorBar } from "@/components/widgets/period-selector-bar";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { availableSlices } from "@/lib/insight/slices";
import { collectRosterAttrs } from "@/lib/insight/slices";
import { normalizePersonId } from "@/lib/metrics/entity";
import { useIcPerson } from "@/queries/ic-dashboard";

/**
 * Global portal bar — the two cross-cutting controls live here so every zone
 * shares one consistent position: the **period** filter and the **slice**
 * (grouping + peer cohort). Both back global state (usePeriod / portal.slice),
 * so all views react. Slice dimensions are derived once from the viewer's whole
 * org (attributes are org-wide, not per-view), keeping the control universal.
 */
export function PortalTopBar() {
  const { period, customRange, setPeriod, setCustomRange } = usePortalPeriod();
  const { personId } = useViewer();
  const tree = useIcPerson(personId ?? "").data ?? null;
  const dims = useMemo(
    () => availableSlices(collectRosterAttrs(tree, normalizePersonId).values()),
    [tree],
  );

  return (
    // Sticky to the scroll container (`SidebarInset` owns the overflow): scope,
    // slice and period apply to whatever is on screen, so they have to stay
    // reachable while reading down a long zone. Opaque background — content
    // scrolling underneath a translucent bar makes both unreadable.
    <div className="sticky top-0 z-20 flex items-center gap-2 border-b bg-background px-4 py-2 md:px-6">
      {/* Opens the context pane wherever it is collapsed — the drawer is the
          only way to reach navigation on a phone (no rail at all), and the only
          way to reach sections on a tablet. Outside the scroller below, so it
          stays put while the controls slide. */}
      <SidebarTrigger className="shrink-0 lg:hidden" />
      {/* Narrow screens keep the three controls on ONE scrollable row: wrapped,
          they stack three deep and a sticky bar then holds 17% of a phone
          viewport for good. Wide screens wrap as before — there is room. */}
      {/* `justify-end` only once there is room to wrap: inside a horizontal
          scroller it pushes the overflow off the START edge, where no scroll
          gesture can reach it — Scope and Slice became unreachable. */}
      <div className="flex min-w-0 flex-1 items-center gap-2 overflow-x-auto md:flex-wrap md:justify-end md:overflow-x-visible">
        <ScopeSelect />
        <SliceSelect dims={dims} />
        <PeriodSelectorBar
          period={period}
          customRange={customRange}
          onPeriodChange={setPeriod}
          onRangeChange={setCustomRange}
        />
      </div>
    </div>
  );
}
