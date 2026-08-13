import { formatMetricNumber, formatMetricValue } from "@/lib/format";
import { computeDelta, formatTileDelta } from "@/lib/metrics/delta";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

export interface MetricComparisons {
  /** "+17%" against this person's own previous period. */
  change: string | null;
  /** "median 512" over the comparison pool. */
  median: string | null;
}

/**
 * The two things a number can be read against, as plain text.
 *
 * Context, not a verdict. A section states both and colours neither: the
 * reader is looking at their own work, and a red number is an accusation the
 * screen is in no position to make — it knows the count, not why.
 *
 * The same pair the headline tiles carry, built the same way, so a metric
 * does not appear to mean one thing on the overview and another one screen
 * deeper.
 */
export function metricComparisons(
  metric: NormalizedMetricResult,
  previous: NormalizedMetricResult | null | undefined,
  entityId: string
): MetricComparisons {
  const data = forEntity(metric, entityId);
  const previousValue = previous ? forEntity(previous, entityId).value : null;
  const delta = computeDelta(
    data.value,
    previousValue,
    metric.computation,
    metric.format
  );
  const median = data.peer?.median ?? null;
  return {
    change: delta ? formatTileDelta(delta) : null,
    median:
      median != null
        ? `median ${
            metric.format === "percent"
              ? formatMetricValue(median, metric.format, metric.unit)
              : formatMetricNumber(median, metric.format)
          }`
        : null,
  };
}
