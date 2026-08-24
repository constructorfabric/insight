import type { MetricBucket } from "@/api/metric-results-client";

export type ReportGranularity = MetricBucket | "quarter" | "year";

/**
 * Whether the client has to add buckets together to answer this.
 *
 * Day, week and month exist server-side, so the server computes each bucket
 * itself — a ratio is that bucket's own ratio, a median that bucket's own
 * median, and every metric in the catalogue can be reported. Quarter and year
 * do not exist there, so they are months added up, and only a metric whose
 * values may be added can be offered at all.
 */
export function needsRollup(granularity: ReportGranularity): boolean {
  return granularity === "quarter" || granularity === "year";
}

/** The bucket to ask the server for, which is not always the one displayed. */
export function requestBucket(granularity: ReportGranularity): MetricBucket {
  return granularity === "quarter" || granularity === "year"
    ? "month"
    : granularity;
}

/** Sorts as text in the order it reads: 2026-03, 2026-Q1, 2026. */
export function bucketLabel(
  bucketStart: string,
  granularity: ReportGranularity,
): string {
  const [year, month] = bucketStart.split("-");
  if (!year || !month) return bucketStart;
  if (granularity === "year") return year;
  if (granularity === "quarter") {
    return `${year}-Q${Math.floor((Number(month) - 1) / 3) + 1}`;
  }
  if (granularity === "month") return `${year}-${month}`;
  return bucketStart;
}

/**
 * Points folded into the requested bucket.
 *
 * A bucket with no reading at all stays absent rather than becoming zero: a
 * zero is a measurement, and the two mean opposite things in a file someone
 * will total. Where no rollup is needed each point stands alone, so this is
 * simply a relabelling.
 */
export function rollUp(
  points: ReadonlyArray<{ bucket_start: string; value: number | null }>,
  granularity: ReportGranularity,
): Map<string, number | null> {
  const out = new Map<string, number | null>();
  const adding = needsRollup(granularity);
  for (const point of points) {
    const key = bucketLabel(point.bucket_start, granularity);
    if (point.value == null) {
      if (!out.has(key)) out.set(key, null);
      continue;
    }
    out.set(key, adding ? (out.get(key) ?? 0) + point.value : point.value);
  }
  return out;
}

const STEP_DAYS: Partial<Record<ReportGranularity, number>> = { day: 1, week: 7 };

// INVARIANT: must agree with the server's `toStartOfWeek(date, 1)` — cells are
// keyed by the `bucket_start` it returns.
function mondayOf(date: Date): Date {
  const monday = new Date(date);
  monday.setUTCDate(monday.getUTCDate() - ((monday.getUTCDay() + 6) % 7));
  return monday;
}

/** Every bucket the period covers, so a gap renders as an empty cell in place. */
export function bucketsInRange(
  from: string,
  to: string,
  granularity: ReportGranularity,
): string[] {
  const labels: string[] = [];
  const step = STEP_DAYS[granularity];
  if (step) {
    const start = new Date(`${from}T00:00:00Z`);
    for (
      let d = granularity === "week" ? mondayOf(start) : start;
      d <= new Date(`${to}T00:00:00Z`);
      d.setUTCDate(d.getUTCDate() + step)
    ) {
      labels.push(d.toISOString().slice(0, 10));
    }
    return labels;
  }
  const end = new Date(`${to.slice(0, 7)}-01T00:00:00Z`);
  for (
    let d = new Date(`${from.slice(0, 7)}-01T00:00:00Z`);
    d <= end;
    d.setUTCMonth(d.getUTCMonth() + 1)
  ) {
    const label = bucketLabel(d.toISOString().slice(0, 10), granularity);
    if (labels.at(-1) !== label) labels.push(label);
  }
  return labels;
}

const iso = (d: Date): string => d.toISOString().slice(0, 10);

/**
 * The days a bucket actually covers in this report, clipped to the requested
 * period.
 *
 * Clipping is the honest part: a report from mid-May to mid-August touches
 * three months of Q2 and Q3 without covering either, and a row labelled
 * "2026-Q2" beside a full quarter's worth of dates would invite the reader to
 * compare it with one.
 */
export function bucketSpan(
  label: string,
  granularity: ReportGranularity,
  range: { from: string; to: string },
): { from: string; to: string } {
  const clip = (from: Date, to: Date) => ({
    from: iso(from) < range.from ? range.from : iso(from),
    to: iso(to) > range.to ? range.to : iso(to),
  });
  if (granularity === "day") return clip(new Date(`${label}T00:00:00Z`), new Date(`${label}T00:00:00Z`));
  if (granularity === "week") {
    const start = new Date(`${label}T00:00:00Z`);
    const end = new Date(start);
    end.setUTCDate(end.getUTCDate() + 6);
    return clip(start, end);
  }
  if (granularity === "year") {
    return clip(
      new Date(`${label}-01-01T00:00:00Z`),
      new Date(`${label}-12-31T00:00:00Z`),
    );
  }
  if (granularity === "quarter") {
    const [year, quarter] = label.split("-Q");
    const firstMonth = (Number(quarter) - 1) * 3;
    const start = new Date(Date.UTC(Number(year), firstMonth, 1));
    const end = new Date(Date.UTC(Number(year), firstMonth + 3, 0));
    return clip(start, end);
  }
  const [year, month] = label.split("-");
  const start = new Date(Date.UTC(Number(year), Number(month) - 1, 1));
  const end = new Date(Date.UTC(Number(year), Number(month), 0));
  return clip(start, end);
}
