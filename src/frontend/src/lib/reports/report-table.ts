import type {
  MetricFormat,
  MetricResult,
  TimeseriesView,
} from "@/api/metric-results-client";
import { roundMetricValue } from "@/lib/format";
import type { ReportPerson } from "@/lib/identities/report-person";
import {
  bucketSpan,
  bucketsInRange,
  rollUp,
  type ReportGranularity,
} from "@/lib/reports/rollup";
import { reportPersonColumns } from "@/lib/reports/roster-columns";
import type { ReportRows } from "@/lib/reports/rows";

export type ReportCell = string | number | null;

export interface ReportTable {
  columns: string[];
  formats: Array<MetricFormat | null>;
  rows: ReportCell[][];
}

export interface ReportInput {
  people: ReadonlyArray<ReportPerson>;
  /** Selected metrics, in the order their columns should appear. */
  metrics: ReadonlyArray<{ metric_key: string; label: string }>;
  results: ReadonlyMap<string, MetricResult>;
  range: { from: string; to: string };
  granularity: ReportGranularity;
  rows?: ReportRows;
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
  if ((input.rows ?? "people") === "repositories") {
    return buildDimensionTable(input, "repository", "Repository");
  }
  const personColumns = reportPersonColumns(input.people);
  const metrics = input.metrics.map((metric) => ({
    ...metric,
    format: input.results.get(metric.metric_key)?.format ?? null,
  }));
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
      ...personColumns.map((column) => column.header),
      "Period",
      "From",
      "To",
      ...metrics.map((metric) => metric.label),
    ],
    formats: [
      ...personColumns.map(() => null),
      null,
      null,
      null,
      ...metrics.map((metric) => metric.format),
    ],
    rows: input.people.flatMap((person) =>
      buckets.map((bucket) => [
        ...personColumns.map((column) => column.of(person)),
        bucket,
        spans.get(bucket)?.from ?? "",
        spans.get(bucket)?.to ?? "",
        ...metrics.map((metric) => {
          const value = cells.get(cellKey(metric.metric_key, person.entityId, bucket));
          return value == null || metric.format == null
            ? value ?? null
            : roundMetricValue(value, metric.format);
        }),
      ]),
    ),
  };
}

/**
 * One row per dimension value per bucket, every person's contribution summed
 * into it.
 *
 * Summing is why only additive metrics reach here: the picker refuses the
 * others in this mode, because a repository's median is not the median of the
 * medians of the people who worked in it. A value carries the dimension's
 * LABEL where the response gave one and its raw value otherwise — the value
 * is an id (a source joined to `owner/repo`) that no reader recognises.
 */
function buildDimensionTable(
  input: ReportInput,
  dimension: string,
  header: string,
): ReportTable {
  const metrics = input.metrics.map((metric) => ({
    ...metric,
    format: input.results.get(metric.metric_key)?.format ?? null,
  }));
  const buckets = bucketsInRange(
    input.range.from,
    input.range.to,
    input.granularity,
  );

  const labels = new Map<string, string>();
  const cells = new Map<string, number>();
  for (const metric of input.metrics) {
    const series = timeseriesOf(input.results.get(metric.metric_key))?.series;
    for (const entry of series ?? []) {
      const dim = entry.dimensions.find((d) => d.key === dimension);
      // A remainder series carries no dimension value to name, and an
      // ungrouped one is not this table's shape.
      if (!dim?.value) continue;
      const label = dim.label?.trim() || labels.get(dim.value) || dim.value;
      labels.set(dim.value, label);
      for (const [bucket, value] of rollUp(entry.points, input.granularity)) {
        if (value == null) continue;
        const key = cellKey(metric.metric_key, dim.value, bucket);
        cells.set(key, (cells.get(key) ?? 0) + value);
      }
    }
  }

  const spans = new Map(
    buckets.map((bucket) => [
      bucket,
      bucketSpan(bucket, input.granularity, input.range),
    ]),
  );
  const values = [...labels.entries()].sort((left, right) =>
    left[1].localeCompare(right[1]),
  );

  return {
    columns: [header, "Period", "From", "To", ...metrics.map((m) => m.label)],
    formats: [null, null, null, null, ...metrics.map((m) => m.format)],
    rows: values.flatMap(([value, label]) =>
      buckets.map((bucket) => [
        label,
        bucket,
        spans.get(bucket)?.from ?? "",
        spans.get(bucket)?.to ?? "",
        ...metrics.map((metric) => {
          const cell = cells.get(cellKey(metric.metric_key, value, bucket));
          return cell == null || metric.format == null
            ? cell ?? null
            : roundMetricValue(cell, metric.format);
        }),
      ]),
    ),
  };
}
