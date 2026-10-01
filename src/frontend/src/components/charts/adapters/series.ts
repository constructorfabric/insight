import type { MetricResult } from "@/api/custom-client";

import { columnReader, labelOf, toNumber } from "./cells";

export const SERIES_LIMIT = 6;
export const OTHER_LABEL = "Other";
const OTHER_KEY = "other";

export interface SeriesKey {
  key: string;
  label: string;
}

export type SeriesRow = { x: unknown } & Record<string, unknown>;

export interface SeriesData {
  rows: SeriesRow[];
  keys: SeriesKey[];
}

export interface PointGroup {
  key: string;
  label: string;
  points: { x: number; y: number; size?: number }[];
}

export function seriesRows(
  result: MetricResult,
  x: string,
  y: string,
  series?: string,
  carry: Record<string, string> = {}
): SeriesData {
  const readX = columnReader(result, x);
  const readY = columnReader(result, y);
  const readSeries = series ? columnReader(result, series) : () => y;
  const carried = Object.entries(carry).map(
    ([key, column]) => [key, columnReader(result, column)] as const
  );

  const kept = keptSeries(
    result.rows.map((row) => [labelOf(readSeries(row)), toNumber(readY(row))])
  );
  const byX = new Map<unknown, SeriesRow>();

  for (const row of result.rows) {
    const at = readX(row);
    const point = byX.get(at) ?? blankRow(at, kept.keys, carried);
    byX.set(at, point);

    const key = kept.keyOf(labelOf(readSeries(row)));
    point[key] = add(point[key], toNumber(readY(row)));

    for (const [carriedKey, read] of carried) {
      point[carriedKey] ??= toNumber(read(row));
    }
  }

  return { rows: [...byX.values()], keys: kept.keys };
}

export interface PairedRow {
  x: unknown;
  y: number | null;
  y2: number | null;
}

export function shareRows(
  result: MetricResult,
  label: string,
  value: string,
  series: string,
  labelLimit?: number
): SeriesData {
  const drawn = keptLabels(
    seriesRows(result, label, value, series),
    labelLimit
  );

  const rows = drawn.rows.map((row) => {
    const total = drawn.keys.reduce(
      (sum, { key }) => sum + (toNumber(row[key]) ?? 0),
      0
    );
    const shares = drawn.keys.map(({ key }) => [
      key,
      total > 0 ? ((toNumber(row[key]) ?? 0) / total) * 100 : 0,
    ]);

    return { x: row.x, ...Object.fromEntries(shares) } as SeriesRow;
  });

  return { rows, keys: drawn.keys };
}

export function pairedRows(
  result: MetricResult,
  x: string,
  y: string,
  y2: string
): PairedRow[] {
  const bars = seriesRows(result, x, y);
  const line = new Map(
    seriesRows(result, x, y2).rows.map((row) => [row.x, toNumber(row.s0)])
  );

  return bars.rows.map((row) => ({
    x: row.x,
    y: toNumber(row.s0),
    y2: line.get(row.x) ?? null,
  }));
}

export function pointGroups(
  result: MetricResult,
  x: string,
  y: string,
  { series, size }: { series?: string; size?: string } = {}
): PointGroup[] {
  const readX = columnReader(result, x);
  const readY = columnReader(result, y);
  const readSize = size ? columnReader(result, size) : undefined;
  const readSeries = series ? columnReader(result, series) : () => y;

  const kept = keptSeries(
    result.rows.map((row) => [labelOf(readSeries(row)), 1])
  );
  const groups = new Map<string, PointGroup>(
    kept.keys.map(({ key, label }) => [key, { key, label, points: [] }])
  );

  for (const row of result.rows) {
    const px = toNumber(readX(row));
    const py = toNumber(readY(row));
    if (px === null || py === null) continue;

    const point = readSize
      ? { x: px, y: py, size: toNumber(readSize(row)) ?? 0 }
      : { x: px, y: py };
    groups.get(kept.keyOf(labelOf(readSeries(row))))?.points.push(point);
  }

  return [...groups.values()].filter((group) => group.points.length > 0);
}

function keptLabels(drawn: SeriesData, limit: number | undefined): SeriesData {
  if (limit === undefined || drawn.rows.length <= limit) return drawn;

  const total = (row: SeriesRow) =>
    drawn.keys.reduce((sum, { key }) => sum + (toNumber(row[key]) ?? 0), 0);
  const kept = new Set(
    [...drawn.rows].sort((a, b) => total(b) - total(a)).slice(0, limit)
  );

  const other: SeriesRow = { x: OTHER_LABEL };
  for (const { key } of drawn.keys) {
    other[key] = drawn.rows
      .filter((row) => !kept.has(row))
      .reduce((sum, row) => sum + (toNumber(row[key]) ?? 0), 0);
  }

  return {
    rows: [...drawn.rows.filter((row) => kept.has(row)), other],
    keys: drawn.keys,
  };
}

function keptSeries(readings: [string, number | null][]) {
  const totals = new Map<string, number>();
  for (const [label, value] of readings) {
    totals.set(label, (totals.get(label) ?? 0) + Math.abs(value ?? 0));
  }

  const labels = [...totals.keys()];
  const largest = new Set(
    [...labels]
      .sort((a, b) => (totals.get(b) ?? 0) - (totals.get(a) ?? 0))
      .slice(0, SERIES_LIMIT)
  );
  const keys: SeriesKey[] = labels
    .filter((label) => largest.has(label))
    .map((label, index) => ({ key: `s${index}`, label }));
  if (labels.length > largest.size) {
    keys.push({ key: OTHER_KEY, label: OTHER_LABEL });
  }

  const byLabel = new Map(keys.map(({ key, label }) => [label, key]));
  const keyOf = (label: string) =>
    largest.has(label) ? (byLabel.get(label) ?? OTHER_KEY) : OTHER_KEY;

  return { keys, keyOf };
}

function blankRow(
  x: unknown,
  keys: SeriesKey[],
  carried: readonly (readonly [string, unknown])[]
): SeriesRow {
  return {
    x,
    ...Object.fromEntries(keys.map(({ key }) => [key, null])),
    ...Object.fromEntries(carried.map(([key]) => [key, null])),
  };
}

function add(sum: unknown, value: number | null): number | null {
  if (value === null) return typeof sum === "number" ? sum : null;

  return (typeof sum === "number" ? sum : 0) + value;
}
