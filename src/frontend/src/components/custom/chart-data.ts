import type { ChartConfig } from "@gears-frontx/ui-kit";
import type { MouseHandlerDataParam } from "recharts";

import type { MetricResult } from "@/api/custom-client";

/**
 * A metric result as a chart reads it: one object per row, keyed by column.
 *
 * A result is columns and positional rows, which no chart library takes. Only
 * the columns a widget draws are carried over, so a wide result does not turn
 * into wide objects.
 */
export function points(
  result: MetricResult,
  keys: string[]
): Record<string, unknown>[] {
  const indexes = keys.map(
    (key) => [key, result.columns.indexOf(key)] as const
  );

  return result.rows.map((row) =>
    Object.fromEntries(indexes.map(([key, at]) => [key, row[at]]))
  );
}

export const CHART_COLORS = 5;

/** One series, painted from the kit's chart palette. */
export function seriesConfig(key: string, index = 0): ChartConfig {
  return {
    [key]: { label: key, color: `var(--chart-${(index % CHART_COLORS) + 1})` },
  };
}

/** A series chart's click, when it lands on a point rather than between them. */
export function pointClick(
  onPointClick: (() => void) | undefined
): ((state: MouseHandlerDataParam) => void) | undefined {
  if (!onPointClick) return undefined;

  // WORKAROUND: Recharts reports a click between points as a null index.
  return (state) => {
    if (state.activeTooltipIndex != null) onPointClick();
  };
}

export type { ChartConfig };
