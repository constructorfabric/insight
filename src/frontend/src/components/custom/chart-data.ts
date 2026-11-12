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

export interface Stacks {
  rows: Record<string, unknown>[];
  series: { key: string; label: string }[];
  total: string;
  top: string;
}

const STACK_TOTAL = "stack-total";
const STACK_TOP = "stack-top";
const STACK_COLORS = 12;

export function stackColor(index: number): string {
  return `var(--chart-${(index % STACK_COLORS) + 1})`;
}

/**
 * Recharts reads a dotted `dataKey` as a path and a chart colour is a CSS
 * custom property, so a series value is never used as a key itself. The keys
 * added here hold a `-`, which no `as_name` can, so none overwrites a column.
 */
export function stacks(
  result: MetricResult,
  x: string,
  y: string,
  series: string
): Stacks {
  const xAt = result.columns.indexOf(x);
  const yAt = result.columns.indexOf(y);
  const seriesAt = result.columns.indexOf(series);

  const label = (row: unknown[]) => String(row[seriesAt] ?? "—");
  const labels = [...new Set(result.rows.map(label))].sort((a, b) =>
    a.localeCompare(b, "en", { numeric: true })
  );
  const keys = new Map(labels.map((name, index) => [name, `stack-${index}`]));
  const bars = new Map<unknown, Record<string, unknown>>();

  for (const row of result.rows) {
    const key = keys.get(label(row))!;

    const bar = bars.get(row[xAt]) ?? { [x]: row[xAt], [STACK_TOTAL]: 0 };
    bars.set(row[xAt], bar);

    const amount = Number(row[yAt]);
    const value = Number.isFinite(amount) ? amount : 0;
    bar[key] = Number(bar[key] ?? 0) + value;
    bar[STACK_TOTAL] = Number(bar[STACK_TOTAL]) + value;
  }

  const ordered = [...keys.values()];
  const rows = [...bars.values()].map((bar) => ({
    ...Object.fromEntries(ordered.map((key) => [key, 0])),
    ...bar,
    [STACK_TOP]: ordered.findLast((key) => Number(bar[key] ?? 0) > 0),
  }));

  return {
    rows,
    series: [...keys].map(([label, key]) => ({ key, label })),
    total: STACK_TOTAL,
    top: STACK_TOP,
  };
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
