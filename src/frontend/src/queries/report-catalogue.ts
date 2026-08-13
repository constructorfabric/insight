import { useQuery } from "@tanstack/react-query";

import {
  queryMetricResults,
  type MetricComputation,
} from "@/api/metric-results-client";
import { useViewer } from "@/auth";
import { reachableMetricKeys } from "@/lib/insight/coverage";
import { MAX_METRICS_PER_REQUEST } from "@/lib/reports/batching";
import { useMetricDefinitionsResponse } from "@/queries/metric-definitions";

/**
 * Which metrics may be totalled, asked of the server rather than assumed.
 *
 * `computation` rides on metric RESULTS, not on the definition listing the
 * picker reads, so it is collected by asking for every catalogue metric over a
 * single day for a single person. The value comes from the definition rather
 * than from the rows, so it arrives whether or not that metric has anything to
 * report in the probed window, and the answer describes THIS installation
 * rather than an assumption about it.
 *
 * A list of metric keys held in the client would drift the first time a metric
 * is added, and would do so silently.
 */
export async function probeComputations(
  metricKeys: string[],
  personId: string,
  day: string,
): Promise<Map<string, MetricComputation>> {
  const out = new Map<string, MetricComputation>();
  for (let i = 0; i < metricKeys.length; i += MAX_METRICS_PER_REQUEST) {
    const batch = metricKeys.slice(i, i + MAX_METRICS_PER_REQUEST);
    const response = await queryMetricResults({
      entity: { type: "person", ids: [personId] },
      period: { from: day, to: day },
      metrics: batch.map((metric_key) => ({
        metric_key,
        views: [{ view: "period" }],
      })),
    });
    for (const result of response.metrics) {
      out.set(result.metric_key, result.computation);
    }
  }
  return out;
}

export function useMetricComputations(): {
  data: Map<string, MetricComputation> | undefined;
  isPending: boolean;
  isError: boolean;
} {
  const { personId } = useViewer();
  const definitions = useMetricDefinitionsResponse();
  // `is_enabled` is not the same question. A metric can be enabled in the
  // catalogue and still be rejected by the results endpoint as "unknown or
  // unavailable", and that rejection is all-or-nothing: one such key fails the
  // whole batch and empties the picker. `reachableMetricKeys` is the existing
  // answer to what this installation actually serves.
  const keys = [...reachableMetricKeys(definitions.data?.metrics ?? [])].sort();
  const day = new Date().toISOString().slice(0, 10);

  const query = useQuery({
    // Keyed by the metrics themselves: one swapped for another leaves the
    // count unchanged, and the cached answer would then describe a set that no
    // longer exists.
    queryKey: ["metric-computations", keys.join(" "), personId],
    queryFn: () => probeComputations(keys, personId ?? "", day),
    enabled: keys.length > 0 && Boolean(personId),
    staleTime: 30 * 60 * 1000,
  });

  return {
    data: query.data,
    isPending: definitions.isPending || query.isPending,
    isError: definitions.isError || query.isError,
  };
}
