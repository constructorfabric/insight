import type { MetricFormat } from "@/api/metric-results-client";
import type { MetricDefinition } from "@/api/metric-definitions-client";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { byFamily } from "@/lib/reports/families";

export interface CompareRow {
  key: string;
  label: string;
  format: MetricFormat;
  unit: string | null;
  values: Array<number | null>;
}

function servesData(metric: MetricDefinition): boolean {
  if (metric.schema_status === "error") return false;
  return metric.origin === "custom" || metric.last_observed_date != null;
}

export function metricsViewCatalog(
  definitions: readonly MetricDefinition[],
  isVisible: (metricKey: string) => boolean,
): MetricDefinition[] {
  const offered = definitions.filter(
    (metric) =>
      metric.entity_type === "person" &&
      metric.is_enabled &&
      servesData(metric) &&
      isVisible(metric.metric_key),
  );

  return byFamily(offered).flatMap((family) => family.metrics);
}

export function chunkKeys(keys: readonly string[], size: number): string[][] {
  const chunks: string[][] = [];
  for (let i = 0; i < keys.length; i += size) {
    chunks.push(keys.slice(i, i + size));
  }
  return chunks;
}

function finiteValue(
  result: NormalizedMetricResult,
  entityId: string,
): number | null {
  const { value } = forEntity(result, entityId);
  return value != null && Number.isFinite(value) ? value : null;
}

export function keysWithData(
  keys: readonly string[],
  byKey: ReadonlyMap<string, NormalizedMetricResult>,
  entityIds: readonly string[],
): string[] {
  return keys.filter((key) => {
    const result = byKey.get(key);
    return result != null && entityIds.some((id) => finiteValue(result, id) != null);
  });
}

export function compareRows(
  keys: readonly string[],
  byKey: ReadonlyMap<string, NormalizedMetricResult>,
  entityIds: readonly string[],
): CompareRow[] {
  return keys.flatMap((key) => {
    const result = byKey.get(key);
    if (!result) return [];

    return [
      {
        key,
        label: result.short_label ?? result.label,
        format: result.format,
        unit: result.unit,
        values: entityIds.map((id) => finiteValue(result, id)),
      },
    ];
  });
}
