import type { MetricResult } from "@/api/custom-client";
import { toNumber } from "@/components/custom/chart-format";

import { columnReader } from "./cells";

interface PulseSummary {
  latest: number | null;
  change: number | null;
  points: (number | null)[];
  since: unknown;
  until: unknown;
}

export function pulseSummary(
  result: MetricResult,
  x: string,
  y: string
): PulseSummary {
  const readX = columnReader(result, x);
  const readY = columnReader(result, y);
  const points = result.rows.map((row) => toNumber(readY(row)));

  const read = points.flatMap((point, index) =>
    point === null ? [] : [{ value: point, at: index }]
  );
  const first = read[0];
  const latest = read.at(-1);
  const change =
    read.length > 1 && first && latest && first.value !== 0
      ? ((latest.value - first.value) / Math.abs(first.value)) * 100
      : null;

  return {
    latest: latest?.value ?? null,
    change,
    points,
    since: first ? readX(result.rows[first.at] ?? []) : undefined,
    until: readX(result.rows.at(-1) ?? []),
  };
}
