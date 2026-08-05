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
 * metrics for the roster (period + peer), plus a previous-period period-only
 * twin for the trend arrows — trends compare values, not standings, so the
 * peer view would be dead weight there (and its rows would tighten chunking
 * for no gain). Both requests chunk large rosters via
 * `useMetricCollectionSet`. Pass a stable `collection` reference (module
 * constant or memo) — it keys the query.
 */
export function useMemberGridData(
  collection: MetricCollectionConfig,
  entity: MetricCollectionEntity,
  range: DateRange,
  period: PeriodValue,
): MemberGridData {
  const currentCollections = useMemo<readonly KeyedCollection[]>(
    () => [{ key: GRID_KEY, collection: projectViews(collection, ["period", "peer"]) }],
    [collection],
  );
  const previousCollections = useMemo<readonly KeyedCollection[]>(
    () => [{ key: GRID_KEY, collection: projectViews(collection, ["period"]) }],
    [collection],
  );

  const current = useMetricCollectionSet(currentCollections, entity, range);
  const previousRange = useMemo(
    () => previousPeriodRange(range, period),
    [range, period],
  );
  const previous = useMetricCollectionSet(
    previousCollections,
    entity,
    previousRange,
  );

  const cur = current.get(GRID_KEY);
  const prev = previous.get(GRID_KEY);

  return {
    byKey: cur?.byKey ?? EMPTY,
    previousByKey: prev?.byKey ?? EMPTY,
    isPending: cur?.isPending ?? false,
    isFetching: (cur?.isFetching ?? false) || (prev?.isFetching ?? false),
    isError: cur?.isError ?? false,
    refetch: () => {
      cur?.refetch();
      prev?.refetch();
    },
  };
}
