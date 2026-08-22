import type { MetricSnapshot } from "@/api/ai-client";
import type {
  SectionTrendPoint,
  SectionTrendSeries,
} from "@/components/portal/section-trend";

export interface TrendSnapshotContext {
  /** What the chart is called on screen. */
  title: string;
  /** "week" / "month" — what one point spans. */
  bucket: string;
  /** Inclusive start of the window, `YYYY-MM-DD`. */
  since: string;
  /** Inclusive end of the window, `YYYY-MM-DD`. */
  until: string;
  /** How many people the rollup covers. */
  people: number;
}

/**
 * The trend chart, as the reader sees it, in the shape the explain endpoint
 * takes.
 *
 * Every line goes over with its own points, because the reading worth
 * explaining on this chart is how the lines move against each other — a single
 * flattened series would lose exactly that.
 */
export function trendSnapshot(
  series: SectionTrendSeries[],
  data: SectionTrendPoint[],
  { title, bucket, since, until, people }: TrendSnapshotContext
): MetricSnapshot {
  return {
    metric_key: series.map((s) => s.key).join(","),
    label: title,
    value: "",
    period: bucket,
    since,
    until,
    delta: "",
    peer: people > 0 ? `Totals across ${people} people` : "",
    help: "",
    trend: [],
    scope: "organisation",
    series: series.map((s) => ({
      label: s.label,
      points: data.map((row) => numberAt(row, s.key)),
    })),
  };
}

function numberAt(row: SectionTrendPoint, key: string): number | null {
  const value = row[key];
  return typeof value === "number" ? value : null;
}
