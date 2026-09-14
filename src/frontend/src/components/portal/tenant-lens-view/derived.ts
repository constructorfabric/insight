import type { MetricBucket, MetricDimension } from "@/api/metric-results-client";
import type { DateRange } from "@/api/period-to-date-range";

/**
 * The bucket a time-based section actually charted, as a reader-facing note.
 * The lens picks day/week/month from the window, so a config title must not
 * promise a period — the note states the one that was served.
 */
export function bucketNote(bucket: MetricBucket | null): string | null {
  switch (bucket) {
    case "day":
      return "Daily buckets";
    case "week":
      return "Weekly buckets";
    case "month":
      return "Monthly buckets";
    default:
      return null;
  }
}

/**
 * Every derived reading of the tenant lens lives here as a pure function over
 * view rows — renderers only lay out what these return, so the math is
 * testable without a DOM or a mocked chart.
 */

/** One breakdown row scoped to the tenant entity. */
export interface DimRow {
  dimensions: MetricDimension[];
  value: number | null;
}

/** One dimensioned timeseries entry scoped to the tenant entity. */
export {
  dayHourMatrix,
  HOUR_BLOCKS,
  WEEKDAY_LABELS,
  type DayHourMatrix,
} from "@/lib/portal/day-hour-matrix";

export interface DimSeries {
  dimensions: MetricDimension[];
  label?: string;
  points: Array<{ bucket_start: string; value: number | null }>;
}

export function dimValue(row: DimRow | DimSeries, key: string): string | null {
  return row.dimensions.find((d) => d.key === key)?.value ?? null;
}

export function dimLabel(row: DimRow | DimSeries, key: string): string | null {
  const dim = row.dimensions.find((d) => d.key === key);
  if (!dim) return null;
  return dim.label?.trim() || dim.value;
}

// ---------------------------------------------------------------------------
// The gate, exactly as silver defines it (github__ci_runs.sql `is_gate`):
// commit-triggered AND carrying a decided outcome. Any client-side gate math
// derived from ci.runs rows must use these constants and nothing else.
// ---------------------------------------------------------------------------
export const GATE_TRIGGERS = ["push", "pull_request", "merge_queue"] as const;
export const GATE_DECIDED_OUTCOMES = ["success", "failure", "timed_out"] as const;
export const GATE_PASS_OUTCOME = "success";

export interface GateStats {
  value: string;
  label: string;
  runs: number;
  gateRuns: number;
  gatePassed: number;
  gateFailed: number;
  /** Percent 0–100, or null when the group has no gate runs. */
  passRate: number | null;
}

/**
 * Gate statistics per value of `key`, from ci.runs breakdown rows that carry
 * at least [key, trigger, outcome]. Rows may be grouped finer — counts sum.
 */
export function gateStatsBy(rows: readonly DimRow[], key: string): GateStats[] {
  const byValue = new Map<string, GateStats>();
  for (const row of rows) {
    const value = dimValue(row, key);
    const trigger = dimValue(row, "trigger");
    const outcome = dimValue(row, "outcome");
    if (value == null || row.value == null || row.value <= 0) continue;
    const got = byValue.get(value) ?? {
      value,
      label: dimLabel(row, key) ?? value,
      runs: 0,
      gateRuns: 0,
      gatePassed: 0,
      gateFailed: 0,
      passRate: null,
    };
    got.runs += row.value;
    const gated =
      trigger != null &&
      outcome != null &&
      (GATE_TRIGGERS as readonly string[]).includes(trigger) &&
      (GATE_DECIDED_OUTCOMES as readonly string[]).includes(outcome);
    if (gated) {
      got.gateRuns += row.value;
      if (outcome === GATE_PASS_OUTCOME) got.gatePassed += row.value;
      else got.gateFailed += row.value;
    }
    byValue.set(value, got);
  }
  return [...byValue.values()].map((s) => ({
    ...s,
    passRate: s.gateRuns > 0 ? (s.gatePassed / s.gateRuns) * 100 : null,
  }));
}

export interface MarginalImpactStep {
  n: number;
  /** Display labels of the pipelines "fixed" so far, worst first. */
  pipelines: string[];
  /** The org gate pass rate if those pipelines' failures had passed. */
  rate: number;
  /** Percentage points gained over the current rate. */
  delta: number;
}

export interface MarginalImpact {
  currentRate: number;
  gateRuns: number;
  steps: MarginalImpactStep[];
}

/**
 * What fixing the worst pipelines would buy: failures of the top-N
 * failing pipelines counted as passes (denominator unchanged). Null when
 * there are no gate runs or nothing fails.
 */
export function marginalImpact(
  rows: readonly DimRow[],
  maxSteps = 3
): MarginalImpact | null {
  const perPipeline = gateStatsBy(rows, "pipeline")
    .filter((s) => s.gateFailed > 0)
    .sort((a, b) => b.gateFailed - a.gateFailed || a.value.localeCompare(b.value));
  const gateRuns = gateStatsBy(rows, "outcome").reduce(
    (sum, s) => sum + s.gateRuns,
    0
  );
  const gatePassed = gateStatsBy(rows, "outcome").reduce(
    (sum, s) => sum + s.gatePassed,
    0
  );
  if (gateRuns === 0 || perPipeline.length === 0) return null;
  const currentRate = (gatePassed / gateRuns) * 100;
  const steps: MarginalImpactStep[] = [];
  let recovered = 0;
  const pipelines: string[] = [];
  for (const [index, stat] of perPipeline.slice(0, maxSteps).entries()) {
    recovered += stat.gateFailed;
    pipelines.push(stat.label);
    const rate = ((gatePassed + recovered) / gateRuns) * 100;
    steps.push({
      n: index + 1,
      pipelines: [...pipelines],
      rate,
      delta: rate - currentRate,
    });
  }
  return { currentRate, gateRuns, steps };
}

// ---------------------------------------------------------------------------
// Series statistics.
// ---------------------------------------------------------------------------

export function mean(values: readonly number[]): number | null {
  if (values.length === 0) return null;
  return values.reduce((sum, v) => sum + v, 0) / values.length;
}

/** Sample standard deviation; null below two observations. */
export function sampleStddev(values: readonly number[]): number | null {
  if (values.length < 2) return null;
  const m = mean(values);
  if (m == null) return null;
  const variance =
    values.reduce((sum, v) => sum + (v - m) ** 2, 0) / (values.length - 1);
  return Math.sqrt(variance);
}

export function medianOf(values: readonly number[]): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1
    ? sorted[mid]
    : (sorted[mid - 1] + sorted[mid]) / 2;
}

export interface TrendPointReading {
  date: string;
  value: number;
}

/**
 * Buckets that fell more than `k` sample deviations below the mean of their
 * own trailing window — "this day was bad by this series' own recent
 * standard", not against a fixed threshold. Needs `minPrior` prior points
 * and a non-degenerate spread, so a flat series flags nothing.
 */
export function trailingOutlierDates(
  points: readonly TrendPointReading[],
  { window = 7, minPrior = 3, k = 2 }: { window?: number; minPrior?: number; k?: number } = {}
): string[] {
  const flagged: string[] = [];
  for (let i = 0; i < points.length; i += 1) {
    const prior = points.slice(Math.max(0, i - window), i).map((p) => p.value);
    if (prior.length < minPrior) continue;
    const m = mean(prior);
    const sd = sampleStddev(prior);
    if (m == null || sd == null || sd === 0) continue;
    if (points[i].value < m - k * sd) flagged.push(points[i].date);
  }
  return flagged;
}

// ---------------------------------------------------------------------------
// Composition math.
// ---------------------------------------------------------------------------

export interface SegmentShare {
  value: string;
  label: string;
  amount: number;
  /** Percent 0–100 of the total. */
  share: number;
}

/** One 100% decomposition of summable rows by a dimension. */
export function decomposeBy(rows: readonly DimRow[], key: string): SegmentShare[] {
  const byValue = new Map<string, { label: string; amount: number }>();
  let total = 0;
  for (const row of rows) {
    const value = dimValue(row, key);
    if (value == null || row.value == null || row.value <= 0) continue;
    const got = byValue.get(value) ?? {
      label: dimLabel(row, key) ?? value,
      amount: 0,
    };
    got.amount += row.value;
    byValue.set(value, got);
    total += row.value;
  }
  if (total <= 0) return [];
  return [...byValue.entries()]
    .map(([value, { label, amount }]) => ({
      value,
      label,
      amount,
      share: (amount / total) * 100,
    }))
    .sort((a, b) => b.amount - a.amount || a.value.localeCompare(b.value));
}

export interface CumulativeRow extends SegmentShare {
  rank: number;
  /** Percent 0–100 including every larger contributor. */
  cumulativeShare: number;
}

/** The same shares, ranked largest-first with a running total. */
export function cumulativeShares(rows: readonly DimRow[], key: string): CumulativeRow[] {
  let running = 0;
  return decomposeBy(rows, key).map((entry, index) => {
    running += entry.share;
    return { ...entry, rank: index + 1, cumulativeShare: Math.min(running, 100) };
  });
}

// ---------------------------------------------------------------------------
// Timeseries reshaping.
// ---------------------------------------------------------------------------

export interface StackedTrendData {
  segments: Array<{ value: string; label: string }>;
  /** One row per bucket; segment values keyed by segment value. */
  rows: Array<{ date: string; values: Record<string, number> }>;
}

/**
 * Dimensioned series → per-bucket rows keyed by segment. `share` converts
 * each bucket to percent-of-bucket-total, dropping empty buckets.
 */
export function stackedTrend(
  series: readonly DimSeries[],
  splitBy: string,
  { share = false }: { share?: boolean } = {}
): StackedTrendData {
  const segments = new Map<string, string>();
  const byDate = new Map<string, Record<string, number>>();
  for (const entry of series) {
    const value = dimValue(entry, splitBy);
    if (value == null) continue;
    if (!segments.has(value)) {
      segments.set(value, dimLabel(entry, splitBy) ?? entry.label ?? value);
    }
    for (const point of entry.points) {
      if (point.value == null) continue;
      const row = byDate.get(point.bucket_start) ?? {};
      row[value] = (row[value] ?? 0) + point.value;
      byDate.set(point.bucket_start, row);
    }
  }
  let rows = [...byDate.entries()]
    .map(([date, values]) => ({ date, values }))
    .sort((a, b) => (a.date < b.date ? -1 : 1));
  if (share) {
    rows = rows.flatMap((row) => {
      const total = Object.values(row.values).reduce((sum, v) => sum + v, 0);
      if (total <= 0) return [];
      return [
        {
          date: row.date,
          values: Object.fromEntries(
            Object.entries(row.values).map(([key, v]) => [key, (v / total) * 100])
          ),
        },
      ];
    });
  }
  return {
    segments: [...segments.entries()].map(([value, label]) => ({ value, label })),
    rows,
  };
}

export interface SmallMultiple {
  value: string;
  label: string;
  points: TrendPointReading[];
}

/** One line per dimension value, plus the shared y-axis ceiling. */
export function smallMultiples(
  series: readonly DimSeries[],
  dimension: string,
  top: number
): { multiples: SmallMultiple[]; max: number } {
  const multiples = series
    .flatMap((entry) => {
      const value = dimValue(entry, dimension);
      if (value == null) return [];
      const points = entry.points
        .filter((p): p is { bucket_start: string; value: number } => p.value != null)
        .map((p) => ({ date: p.bucket_start, value: p.value }));
      if (points.length < 2) return [];
      return [
        {
          value,
          label: dimLabel(entry, dimension) ?? entry.label ?? value,
          points,
          total: points.reduce((sum, p) => sum + p.value, 0),
        },
      ];
    })
    .sort((a, b) => b.total - a.total || a.value.localeCompare(b.value))
    .slice(0, top)
    .map(({ value, label, points }) => ({ value, label, points }));
  const max = Math.max(
    0,
    ...multiples.flatMap((m) => m.points.map((p) => p.value))
  );
  return { multiples, max };
}

export interface HourColumn {
  block: string;
  label: string;
  value: number;
}

export interface HourColumnsData {
  columns: HourColumn[];
  mean: number | null;
  stddev: number | null;
}

/** Per-hour-block readings with the unweighted mean ± σ across blocks. */
export function hourColumns(rows: readonly DimRow[]): HourColumnsData {
  const columns = rows
    .flatMap((row) => {
      const block = dimValue(row, "hour_block");
      if (block == null || row.value == null) return [];
      return [
        { block, label: dimLabel(row, "hour_block") ?? block, value: row.value },
      ];
    })
    .sort((a, b) => a.block.localeCompare(b.block));
  const values = columns.map((c) => c.value);
  return { columns, mean: mean(values), stddev: sampleStddev(values) };
}

// ---------------------------------------------------------------------------
// Two-halves comparison (slope / momentum).
// ---------------------------------------------------------------------------

/**
 * The window split at its midpoint; the second half gets the odd day so the
 * recent half is never the thinner one. Null when either half would be
 * shorter than two days — a one-day "half" is noise dressed as direction.
 */
export function splitDateRange(
  range: DateRange
): { first: DateRange; second: DateRange } | null {
  const from = parseUtcDay(range.from);
  const to = parseUtcDay(range.to);
  if (from == null || to == null || from > to) return null;
  const days = Math.floor((to - from) / DAY_MS) + 1;
  if (days < 4) return null;
  const firstDays = Math.floor(days / 2);
  const firstTo = from + (firstDays - 1) * DAY_MS;
  return {
    first: { from: formatUtcDay(from), to: formatUtcDay(firstTo) },
    second: { from: formatUtcDay(firstTo + DAY_MS), to: formatUtcDay(to) },
  };
}

const DAY_MS = 86_400_000;

function parseUtcDay(value: string): number | null {
  const [year, month, day] = value.split("-").map(Number);
  if (!year || !month || !day) return null;
  const ts = Date.UTC(year, month - 1, day);
  return Number.isNaN(ts) ? null : ts;
}

function formatUtcDay(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`;
}

export interface HalfComparison {
  value: string;
  label: string;
  first: number;
  second: number;
  delta: number;
}

/**
 * Per-dimension-value readings present in BOTH halves — a value observed in
 * only one half has no direction, only an appearance.
 */
export function halvesComparison(
  firstRows: readonly DimRow[],
  secondRows: readonly DimRow[],
  key: string
): HalfComparison[] {
  const first = new Map<string, { label: string; value: number }>();
  for (const row of firstRows) {
    const value = dimValue(row, key);
    if (value == null || row.value == null) continue;
    first.set(value, { label: dimLabel(row, key) ?? value, value: row.value });
  }
  const out: HalfComparison[] = [];
  for (const row of secondRows) {
    const value = dimValue(row, key);
    if (value == null || row.value == null) continue;
    const before = first.get(value);
    if (!before) continue;
    out.push({
      value,
      label: dimLabel(row, key) ?? before.label,
      first: before.value,
      second: row.value,
      delta: row.value - before.value,
    });
  }
  return out.sort(
    (a, b) => Math.abs(b.delta) - Math.abs(a.delta) || a.value.localeCompare(b.value)
  );
}

// ---------------------------------------------------------------------------
// Pairwise panels.
// ---------------------------------------------------------------------------

export interface DumbbellRow {
  value: string;
  label: string;
  left: number;
  right: number;
}

/** Rows with BOTH split readings, widest left-over-right gap first. */
export function dumbbellPairs(
  rows: readonly DimRow[],
  dimension: string,
  splitBy: string,
  left: string,
  right: string
): DumbbellRow[] {
  const byValue = new Map<
    string,
    { label: string; left: number | null; right: number | null }
  >();
  for (const row of rows) {
    const value = dimValue(row, dimension);
    const split = dimValue(row, splitBy);
    if (value == null || split == null || row.value == null) continue;
    const got = byValue.get(value) ?? {
      label: dimLabel(row, dimension) ?? value,
      left: null,
      right: null,
    };
    if (split === left) got.left = row.value;
    if (split === right) got.right = row.value;
    byValue.set(value, got);
  }
  return [...byValue.entries()]
    .flatMap(([value, { label, left: l, right: r }]) =>
      l != null && r != null ? [{ value, label, left: l, right: r }] : []
    )
    .sort((a, b) => b.left - b.right - (a.left - a.right) || a.value.localeCompare(b.value));
}

export interface ScatterPoint {
  value: string;
  label: string;
  x: number;
  y: number;
  size?: number;
}

export interface ScatterData {
  points: ScatterPoint[];
  medianX: number | null;
  medianY: number | null;
}

/** Join x/y(/size) breakdowns by dimension value; a point needs both axes. */
export function scatterPoints(
  xRows: readonly DimRow[],
  yRows: readonly DimRow[],
  sizeRows: readonly DimRow[] | null,
  key: string
): ScatterData {
  const collect = (rows: readonly DimRow[]) => {
    const map = new Map<string, { label: string; value: number }>();
    for (const row of rows) {
      const value = dimValue(row, key);
      if (value == null || row.value == null) continue;
      map.set(value, { label: dimLabel(row, key) ?? value, value: row.value });
    }
    return map;
  };
  const xs = collect(xRows);
  const ys = collect(yRows);
  const sizes = sizeRows ? collect(sizeRows) : null;
  const points: ScatterPoint[] = [];
  for (const [value, x] of xs) {
    const y = ys.get(value);
    if (!y) continue;
    points.push({
      value,
      label: x.label,
      x: x.value,
      y: y.value,
      size: sizes?.get(value)?.value,
    });
  }
  points.sort((a, b) => a.value.localeCompare(b.value));
  return {
    points,
    medianX: medianOf(points.map((p) => p.x)),
    medianY: medianOf(points.map((p) => p.y)),
  };
}

export interface CalloutPair {
  headline: number;
  unweightedMean: number;
  groups: number;
}

/**
 * The org headline against the unweighted mean over a dimension: the gap is
 * how much the busiest groups dominate the headline.
 */
export function calloutPair(
  headline: number | null,
  rows: readonly DimRow[],
  key: string
): CalloutPair | null {
  const values = rows.flatMap((row) =>
    dimValue(row, key) != null && row.value != null ? [row.value] : []
  );
  const m = mean(values);
  if (headline == null || m == null || values.length < 2) return null;
  return { headline, unweightedMean: m, groups: values.length };
}

// ---------------------------------------------------------------------------
// Verdicts.
// ---------------------------------------------------------------------------

export type StabilityVerdict =
  | "solid"
  | "healthy"
  | "erratic"
  | "struggling"
  | "watch";

export interface WeeklyVerdict {
  value: string;
  label: string;
  weeks: number;
  mean: number;
  stddev: number;
  verdict: StabilityVerdict;
}

/**
 * Mean weekly value and volatility per dimension value, resolved to a
 * verdict. The ladder assumes a 0–100 rate where higher is better:
 * spread past 15 points is erratic whatever the mean; a mean under 70 is
 * struggling; ≥95 with ≤5 spread is solid; ≥85 is healthy; the rest bears
 * watching. Histories under `minWeeks` are not judged at all.
 */
export function weeklyVerdicts(
  series: readonly DimSeries[],
  dimension: string,
  minWeeks: number
): { verdicts: WeeklyVerdict[]; thin: number } {
  const verdicts: WeeklyVerdict[] = [];
  let thin = 0;
  for (const entry of series) {
    const value = dimValue(entry, dimension);
    if (value == null) continue;
    const values = entry.points.flatMap((p) => (p.value != null ? [p.value] : []));
    if (values.length < minWeeks) {
      thin += 1;
      continue;
    }
    const m = mean(values);
    const sd = sampleStddev(values) ?? 0;
    if (m == null) continue;
    verdicts.push({
      value,
      label: dimLabel(entry, dimension) ?? entry.label ?? value,
      weeks: values.length,
      mean: m,
      stddev: sd,
      verdict: verdictOf(m, sd),
    });
  }
  verdicts.sort((a, b) => a.mean - b.mean || a.value.localeCompare(b.value));
  return { verdicts, thin };
}

function verdictOf(m: number, sd: number): StabilityVerdict {
  if (sd > 15) return "erratic";
  if (m < 70) return "struggling";
  if (m >= 95 && sd <= 5) return "solid";
  if (m >= 85) return "healthy";
  return "watch";
}
