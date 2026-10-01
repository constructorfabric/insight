import { ChartTooltipContent } from "@gears-frontx/ui-kit";

import { groupedNumber } from "@/components/custom/chart-format";

export const AXIS_TICK = { fill: "var(--muted-foreground)", fontSize: 12 };
export const GRID_STROKE = "var(--grid)";
export const NO_ANIMATION = { isAnimationActive: false } as const;

export function tooltipContent({
  unit = "",
  hideLabel = false,
  labelFormatter,
}: {
  unit?: string;
  hideLabel?: boolean;
  labelFormatter?: (label: unknown) => string;
} = {}) {
  return (
    <ChartTooltipContent
      hideLabel={hideLabel}
      labelFormatter={
        labelFormatter ? (label) => labelFormatter(label) : undefined
      }
      formatter={(value, name) => (
        <span className="flex min-w-40 justify-between gap-5">
          <span className="text-muted-foreground">{String(name ?? "")}</span>
          <strong className="font-medium tabular-nums">
            {groupedNumber(value, unit)}
          </strong>
        </span>
      )}
    />
  );
}
