import type { MetricBucket } from "@/api/metric-results-client";
import type { SectionTrendPoint } from "@/components/portal/section-trend";
import {
  forEntity,
  MAX_PROJECTED_ROWS,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

const DAY_MS = 86_400_000;

const BUCKETS: readonly MetricBucket[] = ["day", "week", "month"];

/**
 * Finest bucket (day → week → month) whose projected rows fit the backend's
 * all-or-nothing row limit — so a large org still gets a coarser trend rather
 * than a failed request.
 *
 * The budget is PER METRIC, not shared across them: the backend checks each
 * view of each metric on its own (`validate_projected_view_limits`) and
 * compiles one query per metric, each carrying its own LIMIT. Dividing the
 * limit by the number of plotted metrics refused windows the backend would
 * have answered — a three-series trend gave up at a third of the roster it
 * could actually chart.
 *
 * `null` is the fourth outcome and the important one: past a certain
 * members × buckets, even monthly does not fit, and returning "month" anyway
 * just sends a request the backend is guaranteed to reject. A suppressed trend
 * with a stated reason beats a 400 the reader has to interpret.
 */
export function pickTrendBucket(
  members: number,
  range: { from: string; to: string },
): MetricBucket | null {
  // A timeseries view answers one row per (member, bucket) plus a total row
  // per member — the backend's own projection, mirrored here.
  const rowsPerBucket = Math.max(1, members);
  const maxBuckets = Math.floor(MAX_PROJECTED_ROWS / rowsPerBucket) - 1;
  if (maxBuckets < 1) return null;

  return BUCKETS.find((bucket) => bucketCount(range, bucket) <= maxBuckets) ?? null;
}

/**
 * Buckets the backend will enumerate for this range, by its rule (weeks start
 * Monday, months on the 1st) rather than by dividing the span: a 365-day
 * window touches 13 months, and days/30.44 says 12. Undercounting means
 * checking a smaller projection than the response actually carries, which
 * spends the headroom this budget keeps below the hard limit.
 */
function bucketCount(range: { from: string; to: string }, bucket: MetricBucket): number {
  const from = Date.parse(`${range.from}T00:00:00Z`);
  const to = Date.parse(`${range.to}T00:00:00Z`);
  if (Number.isNaN(from) || Number.isNaN(to) || to < from) return 1;

  switch (bucket) {
    case "day":
      return Math.round((to - from) / DAY_MS) + 1;
    case "week":
      return Math.round((mondayOf(to) - mondayOf(from)) / (DAY_MS * 7)) + 1;
    case "month":
      return monthIndex(to) - monthIndex(from) + 1;
  }
}

function mondayOf(timestamp: number): number {
  const weekday = new Date(timestamp).getUTCDay();
  return timestamp - ((weekday + 6) % 7) * DAY_MS;
}

function monthIndex(timestamp: number): number {
  const date = new Date(timestamp);
  return date.getUTCFullYear() * 12 + date.getUTCMonth();
}

/**
 * How many of the roster actually contributed to a metric in each bucket.
 *
 * Derived from the same per-entity rows the totals are summed from, so it
 * needs no catalog metric of its own: forty merged pull requests read very
 * differently at four contributors than at twenty, and that second number is
 * already in the response.
 *
 * A bucket a person has a reading in but no value counts as inactive — a
 * measured zero is not a contribution.
 */
export function buildActiveContributorData(
  key: string,
  byKey: Map<string, NormalizedMetricResult>,
  memberIds: readonly string[],
): SectionTrendPoint[] {
  const result = byKey.get(key);
  if (!result) return [];

  const contributorsByDate = new Map<string, Set<string>>();
  for (const id of memberIds) {
    for (const s of forEntity(result, id).series) {
      for (const p of s.points) {
        const contributors =
          contributorsByDate.get(p.bucket_start) ?? new Set<string>();
        if ((p.value ?? 0) > 0) contributors.add(id);
        contributorsByDate.set(p.bucket_start, contributors);
      }
    }
  }

  return [...contributorsByDate.entries()]
    .map(([date, contributors]) => ({ date, active: contributors.size }))
    .sort((a, b) => a.date.localeCompare(b.date));
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
