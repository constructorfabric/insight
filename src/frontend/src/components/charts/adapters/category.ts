import type { MetricResult } from "@/api/custom-client";
import { toNumber } from "@/components/custom/chart-format";

import { OTHER_LABEL, columnReader, labelOf, largest } from "./cells";

interface CategoryRow {
  label: string;
  value: number;
}

interface RadarRow extends CategoryRow {
  target: number | null;
}

export function categoryRows(
  result: MetricResult,
  label: string,
  value: string,
  { positiveOnly = false }: { positiveOnly?: boolean } = {}
): CategoryRow[] {
  const readLabel = columnReader(result, label);
  const readValue = columnReader(result, value);

  const totals = new Map<string, number>();
  for (const row of result.rows) {
    const amount = toNumber(readValue(row));
    if (amount === null) continue;

    const name = labelOf(readLabel(row));
    totals.set(name, (totals.get(name) ?? 0) + amount);
  }

  return [...totals]
    .map(([name, total]) => ({ label: name, value: total }))
    .filter((row) => !positiveOnly || row.value > 0);
}

export function ranked(rows: CategoryRow[], limit?: number): CategoryRow[] {
  const sorted = [...rows].sort((a, b) => b.value - a.value);

  return limit === undefined ? sorted : sorted.slice(0, limit);
}

export function withOther(rows: CategoryRow[], keep: number): CategoryRow[] {
  if (rows.length <= keep) return ranked(rows);

  const kept = largest(rows, (row) => row.value, keep);
  const rest = rows
    .filter((row) => !kept.has(row))
    .reduce((sum, row) => sum + row.value, 0);

  return [...ranked([...kept]), { label: OTHER_LABEL, value: rest }];
}

export function radarRows(
  result: MetricResult,
  label: string,
  value: string,
  target?: string
): RadarRow[] {
  const readTarget = target ? columnReader(result, target) : () => null;
  const readLabel = columnReader(result, label);
  const targets = new Map<string, number | null>();
  for (const row of result.rows) {
    const name = labelOf(readLabel(row));
    if (targets.get(name) == null) targets.set(name, toNumber(readTarget(row)));
  }

  return categoryRows(result, label, value).map((row) => ({
    ...row,
    target: targets.get(row.label) ?? null,
  }));
}

export function progress(
  result: MetricResult,
  value: string,
  max?: string
): number | null {
  const first = result.rows[0];
  if (!first) return null;

  const done = toNumber(columnReader(result, value)(first));
  if (done === null) return null;

  const ceiling = max ? toNumber(columnReader(result, max)(first)) : null;
  const raw = max ? (ceiling ? (done / ceiling) * 100 : 0) : done;

  return Math.min(100, Math.max(0, raw));
}
