import type { DateRange } from "@/api/period-to-date-range";
import {
  downloadBlob,
  metricTimeseriesFilename,
} from "@/components/widgets/metric-views/metric-timeseries-export";
import type { MetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";

type CsvValue = string | number | null | undefined;

const BUCKET_HEADER = {
  day: "Day",
  week: "Week",
  month: "Month",
} as const;

function stringValue(value: string): string {
  return /^[\t\r ]*[=+\-@]/.test(value) ? `'${value}` : value;
}

function csvCell(value: CsvValue): string {
  if (value == null) return "";
  const raw =
    typeof value === "number"
      ? Number.isFinite(value)
        ? String(value)
        : ""
      : stringValue(value);
  return /[",\r\n]/.test(raw) ? `"${raw.replaceAll('"', '""')}"` : raw;
}

function columnHeader(
  model: MetricTimeseriesModel,
  columnLabel: string,
  metricLabel: string
): string {
  if (model.dimensions.length === 0) return metricLabel;
  if (model.metrics.length === 1) return columnLabel;
  return `${columnLabel} — ${metricLabel}`;
}

function csvContent(model: MetricTimeseriesModel): string {
  const header = [
    BUCKET_HEADER[model.bucket],
    ...model.columns.flatMap((column) =>
      model.metrics.map((metric) =>
        columnHeader(model, column.label, metric.label)
      )
    ),
  ];
  const rows = model.buckets.map((bucketStart) => [
    bucketStart,
    ...model.columns.flatMap((column) =>
      model.metrics.map((metric) =>
        column.points.get(metric.metric_key)?.get(bucketStart)
      )
    ),
  ]);
  return [header, ...rows]
    .map((row) => row.map(csvCell).join(","))
    .join("\r\n");
}

export function downloadMetricTimeseriesCsv(
  id: string,
  model: MetricTimeseriesModel,
  range: DateRange
): void {
  const blob = new Blob(["\uFEFF", csvContent(model), "\r\n"], {
    type: "text/csv;charset=utf-8",
  });
  downloadBlob(blob, metricTimeseriesFilename(id, range, "csv"));
}
