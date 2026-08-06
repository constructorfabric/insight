import type { MetricBucket } from "@/api/metric-results-client";
import type { SectionTrendPoint } from "@/components/portal/section-trend";
import {
  forEntity,
  MAX_PROJECTED_ROWS,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

/**
 * Finest bucket (day → week → month) whose projected rows
 * (members × metrics × buckets) fit the backend's all-or-nothing row limit —
 * so a large org still gets a coarser trend rather than a failed request.
 *
 * `null` is the fourth outcome and the important one: past a certain
 * members × metrics × months, even monthly does not fit, and returning "month"
 * anyway just sends a request the backend is guaranteed to reject. A suppressed
 * trend with a stated reason beats a 400 the reader has to interpret.
 */
export function pickTrendBucket(
  members: number,
  metrics: number,
  range: { from: string; to: string },
): MetricBucket | null {
  const days = Math.max(1, daysBetween(range.from, range.to));
  const perBucket = Math.max(1, members * Math.max(1, metrics));
  // Headroom below the hard limit so we never sit exactly on the cliff.
  // NO `Math.max(1, …)` floor here: it used to claim that one bucket always
  // fits, which is false once the roster alone exceeds the limit — 4000 people
  // × 2 metrics is 8000 rows in a SINGLE bucket. That floor turned "impossible"
  // into a confident "week" and a guaranteed 400.
  const maxBuckets = Math.floor((MAX_PROJECTED_ROWS * 0.85) / perBucket);
  if (maxBuckets < 1) return null;
  if (days <= maxBuckets) return "day";
  if (Math.ceil(days / 7) <= maxBuckets) return "week";
  // ~30.44 days per month: overestimating buckets here is the safe direction.
  if (Math.ceil(days / 30.44) <= maxBuckets) return "month";
  return null;
}

function daysBetween(from: string, to: string): number {
  const a = Date.parse(`${from}T00:00:00Z`);
  const b = Date.parse(`${to}T00:00:00Z`);
  if (Number.isNaN(a) || Number.isNaN(b) || b < a) return 1;
  return Math.floor((b - a) / 86_400_000) + 1;
}

/**
 * Sum each metric's per-bucket timeseries points across a roster into a single
 * org/team series per bucket, sorted by date. Shared by every portal view that
 * draws a "team totals over time" chart (Overview, Directions, Collaboration)
 * so the aggregation stays in one place.
 */
export function buildTrendData(
  keys: readonly string[],
  byKey: Map<string, NormalizedMetricResult>,
  memberIds: readonly string[],
): SectionTrendPoint[] {
  const byDate = new Map<string, SectionTrendPoint>();
  for (const key of keys) {
    const r = byKey.get(key);
    if (!r) continue;
    for (const id of memberIds) {
      for (const s of forEntity(r, id).series) {
        for (const p of s.points) {
          const row = byDate.get(p.bucket_start) ?? { date: p.bucket_start };
          row[key] = ((row[key] as number | undefined) ?? 0) + (p.value ?? 0);
          byDate.set(p.bucket_start, row);
        }
      }
    }
  }
  return [...byDate.values()].sort((a, b) =>
    String(a.date).localeCompare(String(b.date)),
  );
}
