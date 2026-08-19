import { HelpCircle } from "lucide-react";

import { ScopeSelect } from "@/components/portal/scope-select";
import { SliceSelect } from "@/components/portal/slice-select";
import { SidebarTrigger } from "@/components/ui/sidebar";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { PeriodSelectorBar } from "@/components/widgets/period-selector-bar";
import { usePortalPeriod } from "@/hooks/use-portal-period";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useCohortOptions } from "@/lib/portal/use-cohort-options";
import { cn } from "@/lib/utils";

/**
 * Global portal bar — the two cross-cutting controls live here so every zone
 * shares one consistent position: the **period** filter and the **slice**
 * (grouping + peer cohort). Both back global state (usePeriod / portal.slice),
 * so all views react. Slice dimensions are derived once from the viewer's whole
 * org (attributes are org-wide, not per-view), keeping the control universal.
 */
export function PortalTopBar() {
  const { activeZone } = useActiveZone();
  // Nothing under Manage reads scope, cohort or period.
  const filtered = activeZone !== "manage";

  return (
    // Sticky to the scroll container (`SidebarInset` owns the overflow): scope,
    // slice and period apply to whatever is on screen, so they have to stay
    // reachable while reading down a long zone. Opaque background — content
    // scrolling underneath a translucent bar makes both unreadable.
    <div
      className={cn(
        "sticky top-0 z-20 flex items-center gap-2 border-b bg-background px-4 py-2 md:px-6",
        // Without the group the trigger is all that is left, and it is
        // `lg:hidden` — an empty strip above the content.
        !filtered && "lg:hidden"
      )}
    >
      {/* Opens the context pane wherever it is collapsed — the drawer is the
          only way to reach navigation on a phone (no rail at all), and the only
          way to reach sections on a tablet. Outside the scroller below, so it
          stays put while the controls slide. */}
      <SidebarTrigger className="shrink-0 lg:hidden" />
      {filtered ? <ViewFilters /> : null}
    </div>
  );
}

/**
 * A component rather than a branch in the bar: it owns the cohort-catalog
 * query, so a zone that does not render it does not fire that query.
 */
function ViewFilters() {
  const { period, customRange, setPeriod, setCustomRange } = usePortalPeriod();
  // The Person zone is about ONE person, and it reads nothing from the org
  // scope. Leaving the control up meant the bar named a different person than
  // the page — a reader saw two names and had to work out which one the
  // numbers belonged to.
  const { activeZone } = useActiveZone();
  const scoped = activeZone !== "person";
  // The server's catalog when it has one, the viewer's roster until then —
  // decided in `useCohortOptions`, which also owns which selection is in
  // effect so this control and the comparison cannot disagree.
  const { dims } = useCohortOptions();

  return (
    // Narrow screens keep the three controls on ONE scrollable row: wrapped,
    // they stack three deep and a sticky bar then holds 17% of a phone viewport
    // for good. Wide screens wrap as before — there is room.
    //
    // `justify-end` only once there is room to wrap: inside a horizontal
    // scroller it pushes the overflow off the START edge, where no scroll
    // gesture can reach it — Scope and Slice became unreachable.
    <div
      role="group"
      aria-label="View filters"
      className="flex min-w-0 flex-1 items-center gap-2 overflow-x-auto md:flex-wrap md:justify-end md:overflow-x-visible"
    >
      {scoped ? <ScopeSelect /> : null}
      <span className="flex shrink-0 items-center gap-1.5">
        <span className="hidden text-xs text-muted-foreground md:inline">
          Cohort
        </span>
        {/* Every "vs median" on every screen is computed against whatever this
            names, and the word alone does not say so. */}
        <TooltipProvider delay={200}>
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  className="cursor-help text-muted-foreground"
                  aria-label="What the cohort controls"
                >
                  <HelpCircle className="size-3.5" />
                </button>
              }
            />
            <TooltipContent
              side="bottom"
              className="max-w-xs text-xs leading-relaxed"
            >
              The people every comparison on screen is made against. "Team
              (all)" compares within the org you are looking at; picking an
              attribute compares each person with the people who share their
              value for it.
            </TooltipContent>
          </Tooltip>
        </TooltipProvider>
        <SliceSelect dims={dims} />
      </span>
      <PeriodSelectorBar
        period={period}
        customRange={customRange}
        onPeriodChange={setPeriod}
        onRangeChange={setCustomRange}
      />
    </div>
  );
}
