import type { MetricResult } from "@/api/custom-client";

import { columnReader, toNumber } from "./cells";

export interface PulseSummary {
  latest: number | null;
  change: number | null;
  points: (number | null)[];
}

export function pulseSummary(result: MetricResult, y: string): PulseSummary {
  const readY = columnReader(result, y);
  const points = result.rows.map((row) => toNumber(readY(row)));
  const readings = points.filter((point): point is number => point !== null);

  const first = readings[0];
  const latest = readings.at(-1) ?? null;
  const change =
    readings.length > 1 && first !== undefined && first !== 0 && latest !== null
      ? ((latest - first) / Math.abs(first)) * 100
      : null;

  return { latest, change, points };
}
