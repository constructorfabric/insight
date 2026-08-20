import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import { useMemo } from "react";

import {
  listMetricDefinitions,
  type MetricDefinition,
  type MetricDefinitionListResponse,
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

/**
 * The whole catalog response, not just its metrics.
 *
 * Shares the one cached query the availability gate and the definitions list
 * already use, so reading a second field off it costs no extra request.
 */
export function useMetricDefinitionsResponse(): UseQueryResult<MetricDefinitionListResponse> {
  return useQuery({
    queryKey: ["metric-definitions"],
    queryFn: listMetricDefinitions,
    staleTime: 5 * 60 * 1000,
  });
}

export interface DeclaredMetricDimensions {
  /**
   * Dimensions each metric declares, or null when the catalog could not be
   * read. Asking for one a metric does not declare is rejected outright, so a
   * caller with no catalog asks for none.
   */
  byMetricKey: ReadonlyMap<string, ReadonlySet<string>> | null;
  isPending: boolean;
}

/**
 * The dimensions each metric will accept as a display dimension.
 *
 * Same gate as `useAvailableMetricKeys`, one level down: a drilldown that asks
 * for an undeclared dimension is a 400, not a request served without it.
 */
export function useDeclaredMetricDimensions(): DeclaredMetricDimensions {
  const { data, isPending } = useQuery({
    queryKey: ["metric-definitions"],
    queryFn: listMetricDefinitions,
    staleTime: 5 * 60 * 1000,
  });
  const byMetricKey = useMemo(
    () =>
      data
        ? new Map(
            data.metrics.map((m) => [
              m.metric_key,
              new Set(m.dimensions) as ReadonlySet<string>,
            ])
          )
        : null,
    [data]
  );
  return { byMetricKey, isPending };
}

export interface AvailableMetricKeys {
  /**
   * What this installation can serve, or null when the catalog could not be
   * read at all — callers then fall back to asking for everything rather than
   * silently showing nothing.
   */
  keys: ReadonlySet<string> | null;
  /** True while the catalog is still loading; callers HOLD their requests. */
  isPending: boolean;
}

/**
 * The metric keys this installation can actually serve.
 *
 * Callers use it to ask only for what exists — see
 * `filterCollectionToAvailable`. Disabled definitions are excluded: the backend
 * rejects them the same way it rejects an unknown key, and one rejected key
 * fails the whole request.
 *
 * `isPending` exists because firing an unfiltered request "just for now" is not
 * free: it is exactly the 400 this gate removes, so a caller waits for the
 * catalog instead of flashing an error and recovering on a second round-trip.
 * The catalog is one shared, five-minute-cached query, so the wait is once per
 * session, not per screen.
 */
export function useAvailableMetricKeys(): AvailableMetricKeys {
  const { data, isPending } = useQuery({
    queryKey: ["metric-definitions"],
    queryFn: listMetricDefinitions,
    staleTime: 5 * 60 * 1000,
  });
  const keys = useMemo(
    () =>
      data
        ? new Set(
            data.metrics.filter((m) => m.is_enabled).map((m) => m.metric_key)
          )
        : null,
    [data]
  );
  return { keys, isPending };
}
