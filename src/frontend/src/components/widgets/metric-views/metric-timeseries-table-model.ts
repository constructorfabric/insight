import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import type {
  MetricTimeseriesTableConfig,
  MetricTimeseriesTableTone,
} from "@/lib/metrics/timeseries-table";
import type { MetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";

type MetricTimeseriesTablePart =
  | {
      kind: "metric";
      metricKey: string;
      metric?: NormalizedMetricResult;
      prefix: string;
      tone: MetricTimeseriesTableTone;
    }
  | { kind: "text"; text: string };

export interface MetricTimeseriesTableColumn {
  key: string;
  label: string;
  parts: MetricTimeseriesTablePart[];
}

function metricColumn(
  metric: NormalizedMetricResult,
  labelSource?: "short"
): MetricTimeseriesTableColumn {
  return {
    key: metric.metric_key,
    label:
      labelSource === "short"
        ? (metric.short_label ?? metric.label)
        : metric.label,
    parts: [
      {
        kind: "metric",
        metricKey: metric.metric_key,
        metric,
        prefix: "",
        tone: "default",
      },
    ],
  };
}

function defaultColumns(
  metrics: NormalizedMetricResult[]
): MetricTimeseriesTableColumn[] {
  return metrics.map((metric) => metricColumn(metric));
}

export function resolveMetricTimeseriesTableColumns(
  model: MetricTimeseriesModel,
  config?: MetricTimeseriesTableConfig
): MetricTimeseriesTableColumn[] {
  if (!config) return defaultColumns(model.metrics);

  const metrics = new Map(
    model.metrics.map((metric) => [metric.metric_key, metric])
  );
  const columns = config.columns.flatMap((column, index) => {
    if ("metric" in column) {
      const metric = metrics.get(column.metric);
      return metric ? [metricColumn(metric, column.labelSource)] : [];
    }

    const hasAvailableMetric = column.template.some(
      (part) => "metric" in part && metrics.has(part.metric)
    );
    if (!hasAvailableMetric) return [];

    return [
      {
        key: `template-${index}`,
        label: column.label,
        parts: column.template.map((part) =>
          "text" in part
            ? { kind: "text" as const, text: part.text }
            : {
                kind: "metric" as const,
                metricKey: part.metric,
                metric: metrics.get(part.metric),
                prefix: part.prefix ?? "",
                tone: part.tone ?? ("default" as const),
              }
        ),
      },
    ];
  });

  return columns.length > 0 ? columns : defaultColumns(model.metrics);
}
