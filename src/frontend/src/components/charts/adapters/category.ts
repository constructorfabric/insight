import type { MetricResult } from "@/api/custom-client";

import { columnReader, labelOf, toNumber } from "./cells";
import { OTHER_LABEL } from "./series";

export interface CategoryRow {
  label: string;
  value: number;
}

export interface RadarRow {
  label: string;
  value: number;
  target: number | null;
}

export interface Progress {
  percent: number | null;
  value: number | null;
  max: number | null;
}

export function categoryRows(
  result: MetricResult,
  label: string,
  value: string,
  {
    positiveOnly = false,
    order = "first",
    limit,
    keep,
  }: {
    positiveOnly?: boolean;
    order?: "first" | "desc";
    limit?: number;
    keep?: number;
  } = {}
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

  const rows = [...totals]
    .map(([name, total]) => ({ label: name, value: total }))
    .filter((row) => !positiveOnly || row.value > 0);
  if (keep !== undefined && rows.length > keep) return withOther(rows, keep);

  const ordered =
    order === "desc" ? rows.sort((a, b) => b.value - a.value) : rows;

  return limit === undefined ? ordered : ordered.slice(0, limit);
}

function withOther(rows: CategoryRow[], keep: number): CategoryRow[] {
  const ranked = [...rows].sort((a, b) => b.value - a.value);
  const rest = ranked.slice(keep).reduce((sum, row) => sum + row.value, 0);

  return [...ranked.slice(0, keep), { label: OTHER_LABEL, value: rest }];
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
): Progress {
  const first = result.rows[0];
  if (!first) return { percent: null, value: null, max: null };

  const done = toNumber(columnReader(result, value)(first));
  const ceiling = max ? toNumber(columnReader(result, max)(first)) : null;
  if (done === null) return { percent: null, value: null, max: ceiling };

  const raw = max ? (ceiling ? (done / ceiling) * 100 : 0) : done;

  return {
    percent: Math.min(100, Math.max(0, raw)),
    value: done,
    max: ceiling,
  };
}
