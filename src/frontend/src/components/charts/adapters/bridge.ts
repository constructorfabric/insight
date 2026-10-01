import type { MetricResult } from "@/api/custom-client";

import { categoryRows } from "./category";

export const TOTAL_LABEL = "Total";

export interface BridgeRow {
  label: string;
  low: number;
  high: number;
  delta: number;
  kind: "increase" | "decrease" | "total";
}

export function bridgeRows(
  result: MetricResult,
  label: string,
  value: string
): BridgeRow[] {
  const steps = categoryRows(result, label, value);
  if (steps.length === 0) return [];

  let running = 0;
  const rows: BridgeRow[] = steps.map(({ label: name, value: delta }) => {
    const start = running;
    running += delta;

    return {
      label: name,
      low: Math.min(start, running),
      high: Math.max(start, running),
      delta,
      kind: delta < 0 ? "decrease" : "increase",
    };
  });

  rows.push({
    label: TOTAL_LABEL,
    low: Math.min(0, running),
    high: Math.max(0, running),
    delta: running,
    kind: "total",
  });

  return rows;
}
