import type { MetricComputation } from "@/api/metric-results-client";
import type { MetricDefinition } from "@/api/metric-definitions-client";
import { NOT_ADDITIVE_REASON } from "@/lib/reports/additive";
import { needsRollup, type ReportGranularity } from "@/lib/reports/rollup";

/**
 * Why a metric cannot be put in this report, or null when it can.
 *
 * Every reason is stated rather than acted on silently: a metric that vanishes
 * from the picker sends the reader hunting for a section they remember, and
 * they cannot tell "not measured here" from "I misremembered the name".
 */
export function unavailableReason(
  metric: Pick<MetricDefinition, "metric_key" | "schema_status" | "last_observed_date" | "origin">,
  granularity: ReportGranularity,
  computationByKey: ReadonlyMap<string, MetricComputation>,
): string | null {
  if (metric.schema_status === "error") {
    return "This metric is not computing on this installation";
  }
  if (metric.origin !== "custom" && metric.last_observed_date == null) {
    return "No data reaches us for this metric yet";
  }
  if (!needsRollup(granularity)) return null;

  const computation = computationByKey.get(metric.metric_key);
  if (computation == null) return null;
  const reason = NOT_ADDITIVE_REASON[computation];
  return reason == null
    ? null
    : `${reason} — pick monthly or finer to include it`;
}
