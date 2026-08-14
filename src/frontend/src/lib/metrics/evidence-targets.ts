import {
  evidenceSelection,
  type MetricEvidenceSelection,
} from "@/api/metric-drilldown-client";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";

export interface EvidenceTarget {
  selection: MetricEvidenceSelection;
  label: string;
}

/**
 * The drillable metrics of a collection, deduplicated, in declaration order.
 *
 * Every entry into the evidence dialog offers this set, so a reader who opens
 * one metric can reach its neighbours without going back to the page.
 */
export function collectionEvidenceTargets(
  metricKeys: readonly string[],
  byKey: ReadonlyMap<string, NormalizedMetricResult>,
  entityId: string,
  period?: { from: string; to: string }
): EvidenceTarget[] {
  const seen = new Set<string>();
  return metricKeys.flatMap((key) => {
    if (seen.has(key)) return [];
    seen.add(key);
    const metric = byKey.get(key);
    if (!metric?.drilldown) return [];
    const selection = evidenceSelection(metric.selection, entityId, period);
    return selection ? [{ selection, label: metric.label }] : [];
  });
}
