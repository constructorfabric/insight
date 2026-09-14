import {
  evidenceSelection,
  personsEvidenceSelection,
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

/**
 * The same figure, narrowed to what the reader clicked.
 *
 * A bar, a segment or a point on a trend answers a slice of the figure above
 * it, so its dialog has to be that slice: each clicked dimension value becomes
 * a filter ON TOP of the selection's own, never instead of it, and a clicked
 * day becomes the period. The narrowed dimensions also join
 * `display_dimensions` — a reader who arrived by clicking one repository
 * should see which repository every row belongs to.
 */
export function narrowedEvidenceSelection(
  metric: NormalizedMetricResult | undefined,
  personIds: readonly string[],
  narrow: {
    filters?: readonly { dimension: string; value: string }[];
    day?: string;
  }
): MetricEvidenceSelection | null {
  if (!metric?.drilldown) return null;
  const canonical = metric.selection;
  if (!canonical) return null;

  const named = (narrow.filters ?? []).filter((f) => f.dimension && f.value);
  const overridden = new Set(named.map((f) => f.dimension));
  const filters = [
    ...canonical.filters.filter((f) => !overridden.has(f.dimension)),
    ...named.map((f) => ({ dimension: f.dimension, values: [f.value] })),
  ];

  return personsEvidenceSelection(
    canonical,
    personIds,
    narrow.day ? { from: narrow.day, to: narrow.day } : undefined,
    filters,
    named.map((f) => f.dimension)
  );
}
