import type { MetricResult } from "@/api/custom-client";
import { toNumber } from "@/components/custom/chart-format";

import {
  OTHER_KEY,
  OTHER_LABEL,
  columnReader,
  labelOf,
  largest,
} from "./cells";

export const SERIES_LIMIT = 6;

export interface SeriesKey {
  key: string;
  label: string;
}

export type SeriesRow = { x: unknown } & Record<string, unknown>;

interface SeriesData {
  rows: SeriesRow[];
  keys: SeriesKey[];
}

interface PointGroup {
  key: string;
  label: string;
  points: { x: number; y: number; size?: number }[];
}

interface PairedRow {
  x: unknown;
  y: number | null;
  y2: number | null;
}

export function seriesRows(
  result: MetricResult,
  x: string,
  y: string,
  series?: string,
  target?: string
): SeriesData {
  const readX = columnReader(result, x);
  const readY = columnReader(result, y);
  const readSeries = series ? columnReader(result, series) : () => y;
  const readTarget = target ? columnReader(result, target) : undefined;

  const kept = keptSeries(
    result.rows.map((row) => [labelOf(readSeries(row)), toNumber(readY(row))])
  );
  const byX = new Map<unknown, SeriesRow>();

  for (const row of result.rows) {
    const at = readX(row);
    const point = byX.get(at) ?? blankRow(at, kept.keys, Boolean(readTarget));
    byX.set(at, point);

    const key = kept.keyOf(labelOf(readSeries(row)));
    point[key] = add(point[key], toNumber(readY(row)));
    if (readTarget) point.target ??= toNumber(readTarget(row));
  }

  return { rows: [...byX.values()], keys: kept.keys };
}

export function rowTotal(row: SeriesRow, keys: SeriesKey[]): number {
  return keys.reduce((sum, { key }) => sum + (toNumber(row[key]) ?? 0), 0);
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
    const total = rowTotal(row, drawn.keys);
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
  const readX = columnReader(result, x);
  const readY = columnReader(result, y);
  const readY2 = columnReader(result, y2);

  const byX = new Map<unknown, PairedRow>();
  for (const row of result.rows) {
    const at = readX(row);
    const point = byX.get(at) ?? { x: at, y: null, y2: null };
    byX.set(at, point);

    point.y = add(point.y, toNumber(readY(row)));
    point.y2 = add(point.y2, toNumber(readY2(row)));
  }

  return [...byX.values()];
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

  const totals = new Map(
    drawn.rows.map((row) => [row, rowTotal(row, drawn.keys)])
  );
  const kept = largest(drawn.rows, (row) => totals.get(row) ?? 0, limit);

  const other: SeriesRow = { x: OTHER_LABEL };
  for (const { key } of drawn.keys) other[key] = 0;
  for (const row of drawn.rows) {
    if (kept.has(row)) continue;

    for (const { key } of drawn.keys) {
      other[key] = (other[key] as number) + (toNumber(row[key]) ?? 0);
    }
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
  const kept = largest(labels, (label) => totals.get(label) ?? 0, SERIES_LIMIT);
  const keys: SeriesKey[] = labels
    .filter((label) => kept.has(label))
    .map((label, index) => ({ key: `s${index}`, label }));
  if (labels.length > kept.size) {
    keys.push({ key: OTHER_KEY, label: OTHER_LABEL });
  }

  const byLabel = new Map(keys.map(({ key, label }) => [label, key]));
  const keyOf = (label: string) =>
    kept.has(label) ? (byLabel.get(label) ?? OTHER_KEY) : OTHER_KEY;

  return { keys, keyOf };
}

function blankRow(
  x: unknown,
  keys: SeriesKey[],
  withTarget: boolean
): SeriesRow {
  return {
    x,
    ...Object.fromEntries(keys.map(({ key }) => [key, null])),
    ...(withTarget ? { target: null } : {}),
  };
}

function add(sum: unknown, value: number | null): number | null {
  if (value === null) return typeof sum === "number" ? sum : null;

  return (typeof sum === "number" ? sum : 0) + value;
}
