import type { ReactNode } from "react";

import { IcViewToggle } from "@/components/ic-view-toggle";
import { PeriodSelectorBar } from "@/components/widgets/period-selector-bar";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { usePeriod } from "@/hooks/use-period";

export interface DashboardHeaderProps {
  title: string;
  subtitle?: string | null;
  person: string;
  hasReports: boolean;
  /** Extra screen-specific controls, rendered before the period selector. */
  actions?: ReactNode;
}

export function DashboardHeader({
  title,
  subtitle,
  person,
  hasReports,
  actions,
}: DashboardHeaderProps) {
  const { period, customRange, setPeriod, setCustomRange } = usePeriod();

  return (
    <header className="sticky top-0 z-20 flex flex-wrap items-center justify-between gap-3 border-b bg-background/95 px-4 py-3 backdrop-blur-sm">
      <div className="flex items-center gap-3">
        <SidebarTrigger />
        <div className="flex flex-col">
          <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
          {subtitle ? (
            <p className="text-xs text-muted-foreground">{subtitle}</p>
          ) : null}
        </div>
        <IcViewToggle person={person} hasReports={hasReports} />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {actions}
        <PeriodSelectorBar
          period={period}
          customRange={customRange}
          onPeriodChange={setPeriod}
          onRangeChange={setCustomRange}
        />
      </div>
    </header>
  );
}
