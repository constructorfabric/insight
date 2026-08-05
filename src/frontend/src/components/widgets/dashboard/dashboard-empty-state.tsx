import { Button } from "@/components/ui/button";
import type { PeriodValue } from "@/types/insight";

const PERIOD_ORDER: PeriodValue[] = ["week", "month", "quarter", "year"];

const PERIOD_LABEL: Record<PeriodValue, string> = {
  week: "Past week",
  month: "Past month",
  quarter: "Past quarter",
  year: "Past year",
};

function widerPeriods(current: PeriodValue): PeriodValue[] {
  const idx = PERIOD_ORDER.indexOf(current);
  if (idx < 0) return [];
  return PERIOD_ORDER.slice(idx + 1);
}

export interface DashboardEmptyStateProps {
  period: PeriodValue;
  onSetPeriod: (period: PeriodValue) => void;
}

export function DashboardEmptyState({
  period,
  onSetPeriod,
}: DashboardEmptyStateProps) {
  const wider = widerPeriods(period);
  const currentLabel = PERIOD_LABEL[period] ?? period;

  return (
    <div className="flex min-h-[60vh] flex-col items-center justify-center gap-4 text-center">
      <div className="flex max-w-md flex-col gap-2">
        <h2 className="text-lg font-semibold tracking-tight">
          No activity in {currentLabel.toLowerCase()}
        </h2>
        <p className="text-sm text-muted-foreground">
          Nothing has been recorded in this period.
          {wider.length > 0 ? " Try a wider one." : ""}
        </p>
      </div>
      {wider.length > 0 ? (
        <div className="flex flex-wrap items-center justify-center gap-2">
          {wider.map((p) => (
            <Button
              key={p}
              variant="outline"
              size="sm"
              onClick={() => onSetPeriod(p)}
            >
              Try {PERIOD_LABEL[p].toLowerCase()}
            </Button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
