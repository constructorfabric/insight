/**
 * Shaping for the ingestion-intensity charts.
 *
 * Pure on purpose: the two things that are easy to get wrong here — the
 * timezone the buckets are read in, and the log axis a zero-baseline bar
 * disappears on — are decided in this file and pinned by its tests, not
 * re-derived inside a component.
 */

import type {
  IngestionGrain,
  IngestionPoint,
  IngestionSeries,
} from "@/api/ingestion-client";

/** The band a `series=total` read comes back under. */
export const TOTAL_KEY = "all";

/**
 * How many `--chart-N` custom properties `index.css` actually defines.
 *
 * Twelve, not twenty-four: the file lists them twice — once on `:root` and
 * again in the dark-mode block — so counting lines rather than distinct indices
 * overshoots, and an index past the last real one resolves to an undefined
 * `var()` that paints every band the fallback colour.
 */
const PALETTE_SIZE = 12;

/**
 * Log-axis baseline, deliberately below 1.
 *
 * A bar is drawn from the axis minimum, so a floor of exactly 1 gives every
 * single-row bucket zero height and the sparse tail of the chart silently
 * vanishes. Sitting the floor under the smallest possible count is what keeps
 * "one row landed here" visible.
 */
export const LOG_FLOOR = 0.5;

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

function pad(value: number): string {
  return value < 10 ? `0${value}` : String(value);
}

/**
 * Bucket string to epoch milliseconds.
 *
 * The server emits `YYYY-MM-DD HH:MM:SS` with no zone marker, and
 * `Date.parse` reads exactly that shape as LOCAL time — which would slide
 * every bucket by the reader's offset. The `Z` is appended explicitly so the
 * instant is the one the server bucketed.
 */
export function bucketToEpoch(bucket: string): number {
  return Date.parse(`${bucket.replace(" ", "T")}Z`);
}

/** Axis and tooltip labels, rendered in UTC via `getUTC*` only. */
export function formatUtcBucket(epoch: number, grain: IngestionGrain): string {
  const at = new Date(epoch);
  const clock = `${pad(at.getUTCHours())}:${pad(at.getUTCMinutes())}`;
  if (grain === "1s") return `${clock}:${pad(at.getUTCSeconds())}`;
  return `${pad(at.getUTCDate())} ${MONTHS[at.getUTCMonth()]} ${clock}`;
}

/** Whole-day label for the wide trend, still UTC. */
export function formatUtcDay(epoch: number): string {
  const at = new Date(epoch);
  return `${pad(at.getUTCDate())} ${MONTHS[at.getUTCMonth()]}`;
}

/**
 * Colour for one entity, as a pure function of its name.
 *
 * Deriving the index from the key rather than from its position in the current
 * result is what makes a connector keep its colour when the window changes,
 * when a neighbour stops ingesting, and across the overview and drill-down
 * pages. Two names can land on the same swatch; that is cosmetic, whereas a
 * colour that moves between renders misreads as a different connector.
 */
export function seriesColorVar(key: string): string {
  // FNV-1a, 32-bit. Any stable hash would do; this one is short and has no
  // dependency.
  let hash = 0x811c9dc5;
  for (let i = 0; i < key.length; i += 1) {
    hash ^= key.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return `var(--chart-${(hash % PALETTE_SIZE) + 1})`;
}

export interface PivotRow {
  /** Bucket start, epoch ms UTC — a numeric axis so gaps stay gaps. */
  epoch: number;
  total: number;
  [key: string]: number;
}

export interface Pivoted {
  rows: PivotRow[];
  /** Band names, widest first, so the stack order matches the legend. */
  keys: string[];
}

/**
 * Flat `{bucket, key, rows}` to one row per bucket with a column per band.
 *
 * No gap filling: the charts plot a numeric time axis, so an absent bucket is
 * an absent bar rather than a category the axis would close up. Filling would
 * also mean inventing up to 38k zero rows for the widest window.
 */
export function pivotIntensity(points: readonly IngestionPoint[]): Pivoted {
  const byBucket = new Map<number, PivotRow>();
  const totals = new Map<string, number>();

  for (const point of points) {
    const epoch = bucketToEpoch(point.bucket);
    if (Number.isNaN(epoch)) continue;
    const row = byBucket.get(epoch) ?? { epoch, total: 0 };
    // A key repeated within a bucket is not something the GROUP BY can
    // produce, but summing keeps the total honest if it ever does.
    row[point.key] = (row[point.key] ?? 0) + point.rows;
    row.total += point.rows;
    byBucket.set(epoch, row);
    totals.set(point.key, (totals.get(point.key) ?? 0) + point.rows);
  }

  const keys = [...totals.entries()]
    // Name breaks the tie so the order is stable for equal totals.
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([key]) => key);

  return {
    rows: [...byBucket.values()].sort((a, b) => a.epoch - b.epoch),
    keys,
  };
}

/** Per-band totals for the connector roster, widest first. */
export function totalsByKey(
  points: readonly IngestionPoint[],
): Array<{ key: string; rows: number }> {
  const totals = new Map<string, number>();
  for (const point of points) {
    totals.set(point.key, (totals.get(point.key) ?? 0) + point.rows);
  }
  return [...totals.entries()]
    .map(([key, rows]) => ({ key, rows }))
    .sort((a, b) => b.rows - a.rows || a.key.localeCompare(b.key));
}

/**
 * Powers of ten spanning the data, for the log axis.
 *
 * Recharts' own log ticks include the fractional floor, which prints `0.5` on
 * an axis counting whole rows.
 */
export function logTicks(maxValue: number): number[] {
  const ticks = [1];
  while (ticks[ticks.length - 1] < maxValue) {
    ticks.push(ticks[ticks.length - 1] * 10);
  }
  return ticks;
}

/** The band label a chart shows: `series=total` has no entity to name. */
export function bandLabel(key: string, series: IngestionSeries): string {
  return series === "total" && key === TOTAL_KEY ? "All connectors" : key;
}

/** `bronze_bamboohr` reads as a database; the lens talks about connectors. */
export function connectorLabel(sourceDatabase: string): string {
  return sourceDatabase.replace(/^bronze_/, "");
}

/** The `scope` value for a connector slug the roster surfaced. */
export function scopeForConnector(connector: string): string {
  return connector.startsWith("bronze_") ? connector : `bronze_${connector}`;
}

const MS_PER_DAY = 86_400_000;

/**
 * Lower bound for a lookback window, pinned to the start of a UTC day.
 *
 * Anchoring to the day rather than to the caller's instant keeps the value
 * stable for a whole day, so a chart re-reading every minute asks for the same
 * window instead of sliding it by a minute each time. `now` is injected: the
 * clock belongs to the caller, and reading it here would put it in a render.
 */
export function lookbackFrom(now: number, days: number): string {
  const startOfDay = Math.floor(now / MS_PER_DAY) * MS_PER_DAY;
  return new Date(startOfDay - days * MS_PER_DAY).toISOString();
}

/** Bucket width in milliseconds, per grain. */
export function bucketMs(grain: IngestionGrain): number {
  return grain === "1s" ? 1_000 : 15 * 60 * 1_000;
}

/**
 * The window widened by half a bucket at each end.
 *
 * A bar on a numeric axis is CENTRED on its value, so the first and last
 * buckets would each have half their width outside the plot — clipped, and
 * overlapping the y-axis labels. Padding by half a bucket puts the whole bar
 * inside without moving where it sits in time.
 */
export function paddedDomain(
  from: string,
  to: string,
  grain: IngestionGrain,
): [number, number] {
  const half = bucketMs(grain) / 2;
  return [Date.parse(from) - half, Date.parse(to) + half];
}
