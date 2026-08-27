import type { MetricComputation } from "@/api/metric-results-client";

/**
 * Whether a metric's values may be added across periods.
 *
 * `distinct_count` is the one that has to be named: it is formatted as an
 * integer and reads as a counter, but the same person is distinct in every
 * month they were active, so summing months overstates a quarter.
 */
export function isAdditive(computation: MetricComputation): boolean {
  return computation === "sum";
}

export const NOT_ADDITIVE_REASON: Record<MetricComputation, string | null> = {
  sum: null,
  ratio: "A ratio over a period is its own numerator over its own denominator",
  median: "A median cannot be rebuilt from the medians of shorter periods",
  percentile: "A percentile cannot be rebuilt from the percentiles of shorter periods",
  stddev: "A spread cannot be rebuilt from the spreads of shorter periods",
  distinct_count: "Someone active in two months is distinct in each of them",
};

/**
 * The catalogue's additive metrics, in catalogue order.
 *
 * `computation` rides on metric RESULTS rather than on the definition
 * listing, so the caller supplies the map a probe collected; a metric the
 * probe did not answer for is left out rather than guessed at.
 */
export function additiveKeys(
  catalogue: ReadonlyArray<{ metric_key: string }>,
  computationByKey: ReadonlyMap<string, MetricComputation>,
): string[] {
  return catalogue
    .filter((metric) => {
      const computation = computationByKey.get(metric.metric_key);
      return computation != null && isAdditive(computation);
    })
    .map((metric) => metric.metric_key);
}
