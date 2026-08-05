import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  listMetricDefinitions,
  type MetricDefinition,
} from "@/api/metric-definitions-client";

export interface MetricDefinitionGroup {
  /** `metric_key` prefix before the first dot (e.g. "git" for "git.commits"). */
  prefix: string;
  metrics: MetricDefinition[];
}

export function groupByKeyPrefix(
  metrics: MetricDefinition[]
): MetricDefinitionGroup[] {
  const groups = new Map<string, MetricDefinition[]>();
  for (const metric of metrics) {
    const dot = metric.metric_key.indexOf(".");
    const prefix =
      dot > 0 ? metric.metric_key.slice(0, dot) : metric.metric_key;
    const bucket = groups.get(prefix);
    if (bucket) {
      bucket.push(metric);
    } else {
      groups.set(prefix, [metric]);
    }
  }
  return [...groups.entries()].map(([prefix, grouped]) => ({
    prefix,
    metrics: grouped,
  }));
}

export function useMetricDefinitions(): UseQueryResult<
  MetricDefinitionGroup[]
> {
  return useQuery({
    queryKey: ["metric-definitions"],
    queryFn: listMetricDefinitions,
    staleTime: 5 * 60 * 1000,
    select: (data) => groupByKeyPrefix(data.metrics),
  });
}
