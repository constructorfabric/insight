import type { MetricResult, TimeseriesView } from "@/api/metric-results-client";
import {
  bucketSpan,
  bucketsInRange,
  rollUp,
  type ReportGranularity,
} from "@/lib/reports/rollup";
import { PERSON_COLUMNS, type ReportPerson } from "@/lib/reports/roster-columns";

export type ReportCell = string | number | null;

export interface ReportTable {
  columns: string[];
  rows: ReportCell[][];
}

export interface ReportInput {
  people: ReadonlyArray<ReportPerson>;
  /** Selected metrics, in the order their columns should appear. */
  metrics: ReadonlyArray<{ metric_key: string; label: string }>;
  results: ReadonlyMap<string, MetricResult>;
  range: { from: string; to: string };
  granularity: ReportGranularity;
}

function timeseriesOf(result: MetricResult | undefined): TimeseriesView | null {
  const view = result?.views.find((v) => v.view === "timeseries");
  return view?.view === "timeseries" ? view : null;
}

const cellKey = (metricKey: string, entityId: string, bucket: string): string =>
  `${metricKey} ${entityId} ${bucket}`;

/**
 * One row per person per bucket, the person's attributes repeated on each.
 *
 * Repeating them is the point: a file normalised into a person table and a
 * measurement table cannot be pivoted, which is the one thing it is opened
 * for.
 */
export function buildReportTable(input: ReportInput): ReportTable {
  const buckets = bucketsInRange(
    input.range.from,
    input.range.to,
    input.granularity,
  );

  const cells = new Map<string, number | null>();
  for (const metric of input.metrics) {
    const series = timeseriesOf(input.results.get(metric.metric_key))?.series;
    for (const entry of series ?? []) {
      // A grouped series carries a person once per group; the report asks for
      // no grouping, so anything dimensioned is not ours to add.
      if (entry.dimensions.length > 0) continue;
      for (const [bucket, value] of rollUp(entry.points, input.granularity)) {
        cells.set(cellKey(metric.metric_key, entry.entity_id, bucket), value);
      }
    }
  }

  const spans = new Map(
    buckets.map((bucket) => [
      bucket,
      bucketSpan(bucket, input.granularity, input.range),
    ]),
  );

  return {
    columns: [
      ...PERSON_COLUMNS.map((column) => column.header),
      "Period",
      "From",
      "To",
      ...input.metrics.map((metric) => metric.label),
    ],
    rows: input.people.flatMap((person) =>
      buckets.map((bucket) => [
        ...PERSON_COLUMNS.map((column) => column.of(person)),
        bucket,
        spans.get(bucket)?.from ?? "",
        spans.get(bucket)?.to ?? "",
        ...input.metrics.map(
          (metric) =>
            cells.get(cellKey(metric.metric_key, person.entityId, bucket)) ??
            null,
        ),
      ]),
    ),
  };
}
