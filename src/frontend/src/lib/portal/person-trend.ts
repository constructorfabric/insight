import type { DateRange } from "@/api/period-to-date-range";
import type { MetricBucket } from "@/api/metric-results-client";
import type { PeriodValue } from "@/types/insight";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

/** How many finished buckets a tile's line shows. */
export const TREND_BUCKETS = 8;

/**
 * The unit the line is drawn in — the same one the reader picked.
 *
 * The number above the line is a month's number when the reader chose months,
 * so the line has to be months too. Anything else asks them to convert in
 * their head on every glance.
 */
export function trendBucket(period: PeriodValue): MetricBucket {
  return period === "week" ? "week" : "month";
}

const BUCKET_DAYS: Record<MetricBucket, number> = {
  day: 1,
  week: 7,
  month: 31,
};

/** The window to ask for: the finished buckets, plus the running one. */
export function trendRange(today: string, bucket: MetricBucket): DateRange {
  const span = BUCKET_DAYS[bucket] * (TREND_BUCKETS + 1);
  const from = new Date(Date.parse(`${today}T00:00:00Z`) - span * 86_400_000)
    .toISOString()
    .slice(0, 10);
  return { from, to: today };
}

/** First day of the bucket `today` falls in — the one still being filled. */
export function runningBucketStart(
  today: string,
  bucket: MetricBucket
): string {
  if (bucket === "month") return `${today.slice(0, 7)}-01`;
  if (bucket === "week") {
    const d = new Date(`${today}T00:00:00Z`);
    // ISO weeks start on Monday; `getUTCDay` calls Sunday 0.
    const back = (d.getUTCDay() + 6) % 7;
    return new Date(d.getTime() - back * 86_400_000).toISOString().slice(0, 10);
  }
  return today;
}

/** Fewest readings that can bend, and so the fewest worth drawing. */
const MIN_READINGS = 3;

/**
 * A metric's finished readings, oldest first — or null when there is no line
 * worth drawing.
 *
 * The running bucket is dropped, and that is the point of this function. A
 * month four days old holds four days of work; drawn beside finished months it
 * reads as a collapse, and the screen would have invented a decline out of the
 * calendar. Nothing on a tile could tell the reader that the last point means
 * something different from every other point.
 *
 * Null below three readings. Two points always draw a straight line, and a
 * straight line reads as a steady trend — but two is also exactly what a
 * metric has when it has only just started being measured, and "we have just
 * begun looking at this" must not be drawn as "this has been rising evenly".
 * Three is the fewest that can bend, and so the fewest that can be read as a
 * shape rather than as a direction the drawing invented.
 */
export function personTrendPoints(
  metric: NormalizedMetricResult | undefined,
  entityId: string,
  runningStart: string
): (number | null)[] | null {
  if (!metric) return null;
  // One series per entity; a person's own is the only one requested.
  const series = forEntity(metric, entityId).series[0]?.points ?? [];
  const points = series
    .filter((p) => p.bucket_start < runningStart)
    .sort((a, b) => a.bucket_start.localeCompare(b.bucket_start))
    .slice(-TREND_BUCKETS)
    .map((p) => p.value ?? null);
  return points.filter((v) => v != null).length >= MIN_READINGS ? points : null;
}
