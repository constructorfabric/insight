import { useMemo } from "react";

import {
  previousPeriodRange,
  type DateRange,
} from "@/api/period-to-date-range";
import {
  projectViews,
  type MetricCollectionConfig,
  type MetricCollectionEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import {
  useMetricCollectionSet,
  type KeyedCollection,
} from "@/queries/metric-results";
import type { PeriodValue } from "@/types/insight";

const GRID_KEY = "member-grid";

export interface MemberGridData {
  /** Current-period value + peer standing per metric key. */
  byKey: Map<string, NormalizedMetricResult>;
  /** Previous-period value per metric key (period view only). */
  previousByKey: Map<string, NormalizedMetricResult>;
  isPending: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: () => void;
}

const EMPTY = new Map<string, NormalizedMetricResult>();

/**
 * Data for a members grid over any metric collection: the collection's
 * metrics for the roster (period + peer) plus the previous period as an extra
 * window on the same request, which is what the trend arrows read. Large
 * rosters chunk via `useMetricCollectionSet`. Pass a stable `collection`
 * reference (module constant or memo) — it keys the query.
 */
export function useMemberGridData(
  collection: MetricCollectionConfig,
  entity: MetricCollectionEntity,
  range: DateRange,
  period: PeriodValue,
): MemberGridData {
  const collections = useMemo<readonly KeyedCollection[]>(
    () => [
      {
        key: GRID_KEY,
        collection: projectViews(collection, ["period", "peer"]),
      },
    ],
    [collection],
  );
  const compareTo = useMemo(
    () => previousPeriodRange(range, period),
    [range, period],
  );

  const set = useMetricCollectionSet(collections, entity, range, compareTo);
  const grid = set.get(GRID_KEY);

  return {
    byKey: grid?.byKey ?? EMPTY,
    previousByKey: grid?.previousByKey ?? EMPTY,
    isPending: grid?.isPending ?? false,
    isFetching: grid?.isFetching ?? false,
    isError: grid?.isError ?? false,
    refetch: () => {
      grid?.refetch();
    },
  };
}
