import {
  entityObserved,
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { quantile } from "@/lib/insight/within-team-peer";

/**
 * Pure stats used by the Directions section library (design §3 rules 1-4, 11).
 * All functions are roster-scoped: callers pass the member-id list of the
 * active scope, never the whole org implicitly.
 */

/** Representative period value: total for sums, median across people otherwise. */
export function representative(
  r: NormalizedMetricResult | undefined,
  ids: readonly string[],
): number | null {
  if (!r) return null;
  const vals = ids
    .map((id) => forEntity(r, id).value)
    .filter((v): v is number => v != null && Number.isFinite(v));
  if (!vals.length) return null;
  if (r.computation === "sum") return vals.reduce((a, b) => a + b, 0);
  return quantile([...vals].sort((a, b) => a - b), 0.5);
}

/**
 * Median across people, regardless of the result's own `computation` kind.
 * Unlike `representative`, this never sums — useful for stat-tile sections
 * that want a per-person median health read on a metric that's normally
 * summed (e.g. a counter's per-person spread), without re-wrapping the
 * result to fake a different computation kind.
 */
export function medianAcross(
  r: NormalizedMetricResult | undefined,
  ids: readonly string[],
): number | null {
  if (!r) return null;
  const vals = ids
    .map((id) => forEntity(r, id).value)
    .filter((v): v is number => v != null && Number.isFinite(v));
  if (!vals.length) return null;
  return quantile([...vals].sort((a, b) => a - b), 0.5);
}

/** Per-active-person mean for a summable metric (denominator = value > 0). */
export function perCapita(r: NormalizedMetricResult, ids: readonly string[]): number {
  let total = 0;
  let active = 0;
  for (const id of ids) {
    const v = forEntity(r, id).value;
    if (v != null && Number.isFinite(v) && v > 0) {
      total += v;
      active += 1;
    }
  }
  return active ? total / active : 0;
}

/** One person's value for a metric, kept together so a figure can name its people. */
export interface EntityValue {
  id: string;
  value: number;
}

/** Finite per-person values of a metric over the scope, in roster order. */
export function entityValues(
  r: NormalizedMetricResult | undefined,
  ids: readonly string[],
): EntityValue[] {
  if (!r) return [];
  return ids.flatMap((id) => {
    const value = forEntity(r, id).value;
    return value != null && Number.isFinite(value) ? [{ id, value }] : [];
  });
}

/**
 * Not exported: nothing outside this module imports the type by name.
 * `DomainLensView` consumes `distribution()`'s return structurally (it reads
 * `.label`/`.range`/`.count`/`.ids` off the inferred row type without naming
 * `DistRow`) — re-export only if a future consumer needs to name the shape.
 */
interface DistRow {
  /** Compact lower-edge tick, e.g. "10" or "1.5k". */
  label: string;
  /** Full band for the tooltip, e.g. "10–15". */
  range: string;
  count: number;
  /** Who landed in the band, so the bar can open the records behind it. */
  ids: string[];
}

/**
 * Frequency distribution of per-person values into evenly-spaced bands.
 * Self-suppression (design rule 11): returns [] below 4 observations, when
 * the maximum is not positive, or when all mass lands in a single bin — a
 * correct-but-meaningless histogram is worse than none.
 */
export function distribution(
  entries: readonly EntityValue[],
  fmt: (n: number) => string,
): DistRow[] {
  if (entries.length < 4) return [];
  const max = Math.max(...entries.map((e) => e.value));
  if (max <= 0) return [];
  const step = chooseStep(max, 14);
  const nBins = Math.max(1, Math.ceil(max / step));
  const bins: string[][] = Array.from({ length: nBins }, () => []);
  for (const { id, value } of entries) {
    // Clamped at both ends: a negative value (a delta-shaped metric, a bad
    // row) would index bins[-1], which lands on a property outside the array
    // and drops the person from the distribution silently.
    bins[Math.min(nBins - 1, Math.max(0, Math.floor(value / step)))]!.push(id);
  }
  if (bins.filter((ids) => ids.length > 0).length < 2) return [];
  return bins.map((ids, i) => ({
    label: fmt(i * step),
    range: `${fmt(i * step)}–${fmt((i + 1) * step)}`,
    count: ids.length,
    ids,
  }));
}

/** What a recharts chart-level click hands back, of what this reads. */
export interface ChartClick {
  activeIndex?: number | string | null;
  activeTooltipIndex?: number | string | null;
  isTooltipActive?: boolean;
}

/**
 * The band a chart-level click landed on, or null.
 *
 * SAFETY: the click handler is bound to the whole chart, including the axis
 * gutters, and recharts reports an index whenever the pointer sits anywhere in
 * the chart. `isTooltipActive` is the chart's own statement that a category is
 * under the pointer, so what opens is always the band the tooltip was naming —
 * never a stale index left over from before the pointer left the chart, where
 * recharts reports `null` and coercing it would land on the first band.
 *
 * WORKAROUND: recharts snaps to the NEAREST category, so a click in the axis
 * gutter opens the band beside it. The tooltip names that band while the
 * pointer is there, so the two agree; nothing here can tell the gutter from
 * the column without hit-testing the plot area by hand.
 */
export function bandAtClick<Row>(
  rows: readonly Row[],
  click: ChartClick,
): Row | null {
  if (!click.isTooltipActive) return null;
  const index = click.activeIndex ?? click.activeTooltipIndex;
  if (index == null) return null;
  const at = Number(index);
  return Number.isInteger(at) ? (rows[at] ?? null) : null;
}

/** Smallest whole 1/2/5·10ⁿ step whose bin count stays at or under `maxBins`. */
export function chooseStep(max: number, maxBins: number): number {
  const mults = [1, 2, 5];
  for (let pow = 0; pow < 12; pow++) {
    for (const m of mults) {
      const step = m * Math.pow(10, pow);
      if (Math.ceil(max / step) <= maxBins) return step;
    }
  }
  return max;
}

/**
 * The busiest tenth of contributors: its share of the total, and who is in it.
 * Null below 4 contributors or on a non-positive total — the same
 * self-suppression the distribution uses, for the same reason.
 */
export function topDecile(
  entries: readonly EntityValue[],
): { share: number; ids: string[] } | null {
  if (entries.length < 4) return null;
  // INVARIANT: ties break on id, so the people named as the busiest tenth are
  // the same set on every render and reload. Roster order is not stable enough
  // to name individuals from.
  const sorted = [...entries].sort(
    (a, b) => b.value - a.value || a.id.localeCompare(b.id),
  );
  const total = sorted.reduce((sum, e) => sum + e.value, 0);
  if (total <= 0) return null;
  const top = sorted.slice(0, Math.max(1, Math.ceil(sorted.length * 0.1)));
  return {
    share: top.reduce((sum, e) => sum + e.value, 0) / total,
    ids: top.map((e) => e.id),
  };
}

/**
 * Compact bin label: 1500 → "1.5k", 1_500_000 → "1.5M", 10 → "10", 2.5 → "2.5".
 *
 * Deliberately NOT `formatAxisTick`, which only abbreviates from 10k: a tick
 * marks a gridline, so "1.5k" there would name a position the line is not
 * exactly on. A bin edge is an exact value off the 1/2/5 ladder, so abbreviating
 * from a thousand loses nothing. The million step exists because without it a
 * million-level bin read "1000k".
 */
export function fmtCompact(n: number): string {
  const abs = Math.abs(n);
  if (abs >= 1_000_000) return `${trim(n / 1_000_000)}M`;
  if (abs >= 1000) return `${trim(n / 1000)}k`;
  return trim(n);
}

function trim(n: number): string {
  return Number.isInteger(n) ? String(n) : n.toFixed(1);
}

/**
 * Whether ANY metric of a lens family has ANY observed entity (design rule 6).
 * Uses `entityObserved` (peer target_value), never zero-filled period sums —
 * this is what distinguishes "measured zero" from "source not ingested".
 */
export function familyObserved(
  byKey: Map<string, NormalizedMetricResult>,
  metricKeys: readonly string[],
  ids: readonly string[],
): boolean {
  for (const key of metricKeys) {
    const r = byKey.get(key);
    if (!r) continue;
    for (const id of ids) {
      if (entityObserved(r, id)) return true;
    }
  }
  return false;
}

/**
 * Domain coverage (Overview design O5): the share of members with at least one
 * OBSERVED metric among `groupKeys` — via `entityObserved` (peer target when
 * present; non-zero period value otherwise, so zero-filled sums never count as
 * observed). Null on an empty roster so callers can suppress.
 */
export function groupCoverage(
  byKey: Map<string, NormalizedMetricResult>,
  groupKeys: readonly string[],
  ids: readonly string[],
): number | null {
  if (!ids.length) return null;
  let covered = 0;
  for (const id of ids) {
    const has = groupKeys.some((k) => {
      const r = byKey.get(k);
      return r != null && entityObserved(r, id);
    });
    if (has) covered += 1;
  }
  return covered / ids.length;
}
