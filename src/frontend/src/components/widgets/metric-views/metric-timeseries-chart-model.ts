import { safeSeriesKey } from "@/components/widgets/metric-views/dimension-series";
import type { MetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import type { MetricTimeseriesChartConfig } from "@/lib/metrics/timeseries-chart";

interface MetricTimeseriesChartSeries {
  key: string;
  metricKey: string;
  columnKey: string;
  colorSeed: string;
  label: string;
  points: Map<string, number | null>;
  total: number | null;
}

export interface MetricTimeseriesChartModel {
  grouped: boolean;
  valueMetric: NormalizedMetricResult;
  series: MetricTimeseriesChartSeries[];
}

export interface TimeseriesNullRun {
  startIndex: number;
  endIndex: number;
}

export function commonNullRuns(
  buckets: string[],
  series: Array<Map<string, number | null>>
): TimeseriesNullRun[] {
  if (series.length === 0) return [];

  const runs: TimeseriesNullRun[] = [];
  let startIndex: number | null = null;
  for (let index = 0; index <= buckets.length; index += 1) {
    const bucket = buckets[index];
    const isMissing =
      bucket !== undefined &&
      series.every((points) => points.get(bucket) == null);
    if (isMissing && startIndex == null) {
      startIndex = index;
    } else if (!isMissing && startIndex != null) {
      runs.push({ startIndex, endIndex: index - 1 });
      startIndex = null;
    }
  }
  return runs;
}

export function shouldCombineTimeseriesMetrics(
  model: MetricTimeseriesModel,
  multiMetric: MetricTimeseriesChartConfig["multiMetric"]
): boolean {
  return (
    multiMetric === "combined" &&
    model.dimensions.length === 0 &&
    model.metrics.length > 1
  );
}

export function buildMetricTimeseriesChartModel(
  model: MetricTimeseriesModel,
  selectedMetricKey: string,
  multiMetric: MetricTimeseriesChartConfig["multiMetric"]
): MetricTimeseriesChartModel | null {
  const selectedMetric =
    model.metrics.find((metric) => metric.metric_key === selectedMetricKey) ??
    model.metrics[0];
  if (!selectedMetric) return null;

  if (shouldCombineTimeseriesMetrics(model, multiMetric)) {
    const column = model.columns[0];
    return {
      grouped: false,
      valueMetric: selectedMetric,
      series: model.metrics.map((metric) => ({
        key: safeSeriesKey(metric.metric_key),
        metricKey: metric.metric_key,
        columnKey: column?.key ?? "",
        colorSeed: metric.metric_key,
        label: metric.label,
        points: column?.points.get(metric.metric_key) ?? new Map(),
        total: column?.totals.get(metric.metric_key) ?? null,
      })),
    };
  }

  return {
    grouped: model.dimensions.length > 0,
    valueMetric: selectedMetric,
    series: model.columns.map((column) => ({
      key: column.key,
      metricKey: selectedMetric.metric_key,
      columnKey: column.key,
      colorSeed: column.colorSeed,
      label: column.label,
      points: column.points.get(selectedMetric.metric_key) ?? new Map(),
      total: column.totals.get(selectedMetric.metric_key) ?? null,
    })),
  };
}
