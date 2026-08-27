import type { MetricComputation } from "@/api/metric-results-client";
import type { MetricDefinition } from "@/api/metric-definitions-client";
import { NOT_ADDITIVE_REASON } from "@/lib/reports/additive";
import { needsRollup, type ReportGranularity } from "@/lib/reports/rollup";
import type { ReportRows } from "@/lib/reports/rows";

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
  rows: ReportRows = "people",
): string | null {
  if (metric.schema_status === "error") {
    return "This metric is not computing on this installation";
  }
  if (metric.origin !== "custom" && metric.last_observed_date == null) {
    return "No data source is connected for this metric yet";
  }
  // Rows that are not people are people's values added up, so the additivity
  // question arrives at every granularity rather than only at the ones the
  // client rolls up itself.
  const adding = rows !== "people" || needsRollup(granularity);
  if (!adding) return null;

  const computation = computationByKey.get(metric.metric_key);
  // Unknown is not the same as additive. The probe may still be in flight, and
  // treating silence as permission is how a ratio ends up summed into a
  // quarter — the one thing this whole path exists to prevent.
  if (computation == null) {
    return "Still checking whether this can be totalled over a period";
  }
  const reason = NOT_ADDITIVE_REASON[computation];
  if (reason == null) return null;
  return rows === "people"
    ? `${reason} — pick monthly or finer to include it`
    : `${reason} — it cannot be totalled per repository`;
}
