import { keepPreviousData, useQueries, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import {
  queryMetricResults,
  type MetricRequest,
} from "@/api/metric-results-client";
import {
  previousPeriodRange,
  type DateRange,
} from "@/api/period-to-date-range";
import {
  buildMetricCollectionRequest,
  chunkEntityIds,
  entityChunkSize,
  mergeNormalizedResults,
  normalizeMetricResults,
  type MetricCollectionConfig,
  type MetricCollectionEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import type { PeriodValue } from "@/types/insight";

export interface MetricCollectionOptions {
  /**
   * When set, a twin query fetches the same collection over the previous
   * period of the same kind (the period value drives week/month/quarter/year
   * shift semantics in `previousPeriodRange`). Consumers derive deltas from
   * `previousByKey`.
   */
  previousPeriod?: PeriodValue;
  keepPreviousData?: boolean;
}

export interface MetricCollectionResult {
  byKey: Map<string, NormalizedMetricResult>;
  previousByKey: Map<string, NormalizedMetricResult> | null;
  isPending: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: () => void;
}

function canonicalEntityIds(entity: MetricCollectionEntity): string[] {
  const ids =
    entity.type === "person"
      ? entity.ids.map(normalizePersonId)
      : entity.ids.map((id) => id.trim());
  return [...new Set(ids.filter(Boolean))].sort();
}

function queryKeyFor(
  entity: MetricCollectionEntity,
  ids: string[],
  range: DateRange,
  metrics: MetricRequest[]
) {
  // The derived `metrics` array rides in the key, so key and payload are
  // provably coherent — no hand-maintained collection identity to forget to
  // bump. TanStack hashes the key structurally.
  return [
    "metric-results",
    entity.type,
    ids,
    range.from,
    range.to,
    metrics,
  ] as const;
}

export function useMetricCollection(
  collection: MetricCollectionConfig,
  entity: MetricCollectionEntity,
  range: DateRange,
  options?: MetricCollectionOptions
): MetricCollectionResult {
  const ids = canonicalEntityIds(entity);
  const request = buildMetricCollectionRequest(
    collection,
    { type: entity.type, ids },
    range
  );
  // An empty entity list is not a request the backend can answer — it rejects
  // `entity.ids: []` with 400 invalid_argument. So the query stays disabled,
  // and because `refetch()` bypasses `enabled`, the `refetch` and `isError`
  // this hook returns have to respect the same flag.
  const enabled = ids.length > 0 && Boolean(range.from && range.to);

  const current = useQuery({
    queryKey: queryKeyFor(entity, ids, range, request.metrics),
    queryFn: () => queryMetricResults(request),
    enabled,
    placeholderData: options?.keepPreviousData ? keepPreviousData : undefined,
  });

  const previousRange = options?.previousPeriod
    ? previousPeriodRange(range, options.previousPeriod)
    : null;
  const previousRequest = previousRange
    ? buildMetricCollectionRequest(
        collection,
        { type: entity.type, ids },
        previousRange
      )
    : null;
  const previous = useQuery({
    // Sentinel key when no previous period is requested: the disabled twin
    // must never alias the current query's cache entry.
    queryKey: previousRequest
      ? queryKeyFor(
          entity,
          ids,
          previousRange ?? range,
          previousRequest.metrics
        )
      : (["metric-results", "previous-disabled"] as const),
    queryFn: () => queryMetricResults(previousRequest ?? request),
    enabled: enabled && previousRequest !== null,
  });

  const hasPrevious = previousRequest !== null;
  const byKey = useMemo(
    () => normalizeMetricResults(current.data?.metrics),
    [current.data]
  );
  // Deltas pair two periods; a failed twin yields "no delta" rather than a
  // silently mispaired one. Both queries reset together on a period change, so
  // the previous twin is absent (not stale) while it reloads — nothing to
  // mispair against.
  const previousUsable = hasPrevious && !previous.isError;
  const previousData = previousUsable ? previous.data : undefined;
  const previousByKey = useMemo(
    () => (previousData ? normalizeMetricResults(previousData.metrics) : null),
    [previousData]
  );

  return {
    byKey,
    previousByKey,
    isPending: current.isPending && enabled,
    isFetching: current.isFetching || (hasPrevious && previous.isFetching),
    // Defensive: `ids` and `range` both ride in the query key, so today a
    // disabled query cannot be holding an error from an enabled one. Kept so
    // that a future key change cannot resurrect "Unable to load" for a
    // collection we are deliberately not asking about.
    isError: enabled && current.isError,
    refetch: () => {
      // `refetch()` ignores `enabled` in react-query, so this guard is what
      // stops a Retry on an unresolved roster from POSTing `entity.ids: []` —
      // see the note on `enabled` above.
      if (!enabled) return;
      void current.refetch();
      if (hasPrevious) void previous.refetch();
    },
  };
}

export interface KeyedCollection {
  key: string;
  collection: MetricCollectionConfig;
}

/** True while any collection in the set still has no data — the screen-level
 *  loading gate (a period change mints new query keys, so it re-trips). */
export function collectionSetPending(
  set: Map<string, MetricCollectionResult>
): boolean {
  return [...set.values()].some((result) => result.isPending);
}

/**
 * One query per collection for a dynamic list (e.g. every metrics-backed
 * group in the registry) — `useQueries`, so the list length can change
 * without violating hook rules. No previous-period twin here; only the KPI
 * row compares periods.
 */
export function useMetricCollectionSet(
  collections: readonly KeyedCollection[],
  entity: MetricCollectionEntity,
  range: DateRange
): Map<string, MetricCollectionResult> {
  const ids = canonicalEntityIds(entity);
  const enabled = ids.length > 0 && Boolean(range.from && range.to);

  // Large rosters are chunked so a period+peer collection over N entities
  // never exceeds the backend's all-or-nothing projected-row limit; chunk
  // results merge back into one collection result per key.
  const requests = collections.flatMap(({ key, collection }) => {
    const chunkSize = entityChunkSize(collection);
    const chunks = chunkSize === null ? [ids] : chunkEntityIds(ids, chunkSize);
    return chunks.map((chunkIds) => ({
      key,
      request: buildMetricCollectionRequest(
        collection,
        { type: entity.type, ids: chunkIds },
        range
      ),
      chunkIds,
    }));
  });

  const results = useQueries({
    queries: requests.map(({ request, chunkIds }) => ({
      queryKey: queryKeyFor(entity, chunkIds, range, request.metrics),
      queryFn: () => queryMetricResults(request),
      enabled,
    })),
  });

  const out = new Map<string, MetricCollectionResult>();
  const chunkMaps = new Map<
    string,
    Array<Map<string, NormalizedMetricResult>>
  >();
  requests.forEach(({ key }, index) => {
    const query = results[index];
    if (!query) return;
    const maps = chunkMaps.get(key) ?? [];
    maps.push(normalizeMetricResults(query.data?.metrics));
    chunkMaps.set(key, maps);
    const existing = out.get(key);
    // Same guard as the single-collection hook: a disabled chunk has no valid
    // request to send, and `refetch()` would send it anyway.
    const refetch = () => {
      if (enabled) void query.refetch();
    };
    out.set(key, {
      byKey: new Map(),
      previousByKey: null,
      isPending: (existing?.isPending ?? false) || (query.isPending && enabled),
      isFetching: (existing?.isFetching ?? false) || query.isFetching,
      isError: (existing?.isError ?? false) || (enabled && query.isError),
      // Chunks of the same collection share a key; refetch fans out to all.
      refetch: existing
        ? () => {
            existing.refetch();
            refetch();
          }
        : refetch,
    });
  });
  for (const [key, maps] of chunkMaps) {
    const entry = out.get(key);
    if (entry) entry.byKey = mergeNormalizedResults(maps);
  }
  return out;
}
