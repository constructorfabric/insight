import { useMemo } from "react";

import type { DateRange } from "@/api/period-to-date-range";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { chunkKeys } from "@/lib/portal/metrics-view";
import {
  useMetricCollectionSet,
  type KeyedCollection,
} from "@/queries/metric-results";

const MAX_METRICS_PER_REQUEST = 50;

export interface MetricsViewData {
  byKey: Map<string, NormalizedMetricResult>;
  isPending: boolean;
  isError: boolean;
  refetch: () => void;
}

export function useMetricsViewData(
  metricKeys: readonly string[],
  personIds: string[],
  range: DateRange,
): MetricsViewData {
  const collections = useMemo<readonly KeyedCollection[]>(
    () =>
      chunkKeys(metricKeys, MAX_METRICS_PER_REQUEST).map((chunk, index) => ({
        key: `metrics-view-${index}`,
        collection: {
          metrics: chunk.map((key) => ({ key, views: [{ view: "period" }] })),
        },
      })),
    [metricKeys],
  );
  const entity = useMemo(
    () => ({ type: "person" as const, ids: personIds }),
    [personIds],
  );

  const set = useMetricCollectionSet(collections, entity, range);

  const results = [...set.values()];

  return {
    byKey: new Map(results.flatMap((result) => [...result.byKey])),
    isPending: results.some((result) => result.isPending),
    isError: results.some((result) => result.isError),
    refetch: () => {
      for (const result of results) result.refetch();
    },
  };
}
