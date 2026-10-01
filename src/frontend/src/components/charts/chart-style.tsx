import { ChartTooltipContent } from "@gears-frontx/ui-kit";

import { groupedNumber } from "@/components/custom/chart-format";

export const AXIS_TICK = { fill: "var(--muted-foreground)", fontSize: 12 };
export const GRID_STROKE = "var(--grid)";
export const LABEL_CHARS = 12;

export function cut(label: unknown, chars = LABEL_CHARS): string {
  const text = String(label ?? "");

  return text.length > chars ? `${text.slice(0, chars)}…` : text;
}

export function tooltipContent({
  unit = "",
  hideLabel = false,
  labelFormatter,
  pick,
}: {
  unit?: string;
  hideLabel?: boolean;
  labelFormatter?: (label: unknown) => string;
  pick?: (row: Record<string, unknown>) => unknown;
} = {}) {
  return (
    <ChartTooltipContent
      hideLabel={hideLabel}
      labelFormatter={
        labelFormatter ? (label) => labelFormatter(label) : undefined
      }
      formatter={(value, name, item) => (
        <span className="flex min-w-40 justify-between gap-5">
          <span className="text-muted-foreground">{String(name ?? "")}</span>
          <strong className="font-medium tabular-nums">
            {groupedNumber(
              pick
                ? pick((item.payload ?? {}) as Record<string, unknown>)
                : value,
              unit
            )}
          </strong>
        </span>
      )}
    />
  );
}
