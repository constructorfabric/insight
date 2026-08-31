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
  filterCollectionByKey,
  filterCollectionToAvailable,
  entityChunkSize,
  mergeNormalizedResults,
  normalizeMetricResults,
  projectComparison,
  projectPrimary,
  type MetricCollectionConfig,
  type MetricCollectionEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import { metricVisible } from "@/lib/portal/nav-policy";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useAvailableMetricKeys } from "@/queries/metric-definitions";
import type { PeriodValue } from "@/types/insight";

/**
 * The install's metric gate, as a predicate over metric keys.
 *
 * Applied to every collection request, so a metric this install hides — or
 * marks planned while the reader has planned sections off — reaches no surface
 * at all: not a tile, not an attention row, not a heatmap column, not a
 * drilldown block. Surfaces already render only the keys that came back, so
 * they need no gate of their own; the ones that draw a shell around a missing
 * metric (a section title, a card) gate their own composition instead.
 */
function useMetricGate(): (metricKey: string) => boolean {
  const showPlanned = usePortalShowPlanned();
  return useMemo(
    () => (metricKey: string) => metricVisible(metricKey, showPlanned),
    [showPlanned]
  );
}

export interface MetricCollectionOptions {
  /**
   * When set, the request carries the previous period of the same kind as an
   * extra window (the period value drives week/month/quarter/year shift
   * semantics in `previousPeriodRange`). Consumers derive deltas from
   * `previousByKey`. Shorthand for a `compareTo` range.
   */
  previousPeriod?: PeriodValue;
  /**
   * The window to compare against, served in the same request and read back
   * through `previousByKey`. Ignored when `previousPeriod` is set, which is
   * sugar for it.
   */
  compareTo?: DateRange;
  keepPreviousData?: boolean;
}

export interface MetricCollectionResult {
  byKey: Map<string, NormalizedMetricResult>;
  /** The comparison window's values, or null when none was requested. */
  previousByKey: Map<string, NormalizedMetricResult> | null;
  isPending: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: () => void;
}

function canonicalEntityIds(entity: MetricCollectionEntity): string[] {
  // The tenant entity names nobody: the backend derives the organization from
  // the session, so there are no ids to canonicalize (or to gate `enabled` on).
  if (entity.type === "tenant") return [];
  const ids = entity.ids.map(normalizePersonId);
  return [...new Set(ids.filter(Boolean))].sort();
}

/** Whether the entity selection itself is answerable (see `enabled` below). */
function entitySelected(
  entity: MetricCollectionEntity,
  ids: string[]
): boolean {
  return entity.type === "tenant" || ids.length > 0;
}

function queryKeyFor(
  entity: MetricCollectionEntity,
  ids: string[],
  range: DateRange,
  metrics: MetricRequest[],
  compareTo?: DateRange
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
    compareTo ?? null,
  ] as const;
}

export function useMetricCollection(
  collection: MetricCollectionConfig,
  entity: MetricCollectionEntity,
  range: DateRange,
  options?: MetricCollectionOptions
): MetricCollectionResult {
  const ids = canonicalEntityIds(entity);
  // Ask only for metrics this installation's catalog offers: the backend
  // rejects the WHOLE request over one unknown key, so a compiled-in key that
  // a tenant does not have would blank the screen instead of its own tile.
  const catalog = useAvailableMetricKeys();
  const gate = useMetricGate();
  const asked = filterCollectionByKey(
    filterCollectionToAvailable(collection, catalog.keys),
    gate
  );
  const canonicalEntity: MetricCollectionEntity =
    entity.type === "person" ? { type: "person", ids } : { type: "tenant" };
  // The comparison window rides along inside the request instead of a twin
  // one: it reads the same rows the primary aggregate already scans, so a delta
  // arrow no longer costs a second round trip, a second authorization and a
  // second query slot (#2651).
  const compareTo = options?.previousPeriod
    ? previousPeriodRange(range, options.previousPeriod)
    : options?.compareTo;
  const request = buildMetricCollectionRequest(
    asked,
    canonicalEntity,
    range,
    compareTo
  );
  // Neither an empty entity list nor an empty metric list is a request the
  // backend can answer — it rejects both with 400 invalid_argument. So the
  // query stays disabled, and because `refetch()` bypasses `enabled`, the
  // `refetch` and `isError` this hook returns have to respect the same flag.
  const enabled =
    entitySelected(entity, ids) &&
    request.metrics.length > 0 &&
    !catalog.isPending &&
    Boolean(range.from && range.to);

  const current = useQuery({
    queryKey: queryKeyFor(entity, ids, range, request.metrics, compareTo),
    queryFn: () => queryMetricResults(request),
    enabled,
    placeholderData: options?.keepPreviousData ? keepPreviousData : undefined,
  });

  // INVARIANT: both projections read the RAW response. A compared breakdown
  // groups over both windows at once, so filtering the primary first would hide
  // the groups that belong only to the comparison window — the very rows it
  // exists to carry.
  const served = useMemo(
    () => normalizeMetricResults(current.data?.metrics),
    [current.data]
  );
  const byKey = useMemo(() => projectPrimary(served), [served]);
  // The comparison window's values, read as if they had been their own request
  // — a period and its comparison now stand or fall together, so there is no
  // mispairing to guard against.
  const previousByKey = compareTo ? projectComparison(served) : null;

  return {
    byKey,
    previousByKey,
    // Pending while the catalog resolves too: the request is coming, so the
    // screen must show a skeleton rather than an empty state it would replace
    // a moment later.
    isPending:
      (current.isPending && enabled) ||
      (entitySelected(entity, ids) && catalog.isPending),
    isFetching: current.isFetching,
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
 * without violating hook rules. `compareTo` rides every request in the set and
 * reads back per collection through `previousByKey`.
 */
export function useMetricCollectionSet(
  collections: readonly KeyedCollection[],
  entity: MetricCollectionEntity,
  range: DateRange,
  compareTo?: DateRange
): Map<string, MetricCollectionResult> {
  const ids = canonicalEntityIds(entity);
  const catalog = useAvailableMetricKeys();
  const gate = useMetricGate();
  const enabled =
    entitySelected(entity, ids) &&
    !catalog.isPending &&
    Boolean(range.from && range.to);

  // Large rosters are chunked so a period+peer collection over N entities
  // never exceeds the backend's all-or-nothing projected-row limit; chunk
  // results merge back into one collection result per key. A tenant entity
  // names nobody, so it is always exactly one "chunk".
  const requests = collections.flatMap(({ key, collection: raw }) => {
    // Same catalog and install gates as `useMetricCollection` — see the notes there.
    const collection = filterCollectionByKey(
      filterCollectionToAvailable(raw, catalog.keys),
      gate
    );
    const chunkSize = entityChunkSize(collection);
    const chunks =
      entity.type === "tenant" || chunkSize === null
        ? [ids]
        : chunkEntityIds(ids, chunkSize);
    return chunks.map((chunkIds) => {
      const request = buildMetricCollectionRequest(
        collection,
        entity.type === "person"
          ? { type: "person", ids: chunkIds }
          : { type: "tenant" },
        range,
        compareTo
      );
      return {
        key,
        request,
        chunkIds,
        // Per-request, not just the shared gate: a collection the catalog
        // covers none of filters down to `metrics: []`, which is itself a 400.
        // It also decides `isPending`/`isError` below — a query that will never
        // fire must not report itself as forever loading.
        active: enabled && request.metrics.length > 0,
      };
    });
  });

  const results = useQueries({
    queries: requests.map(({ request, chunkIds, active }) => ({
      queryKey: queryKeyFor(entity, chunkIds, range, request.metrics, compareTo),
      queryFn: () => queryMetricResults(request),
      enabled: active,
    })),
  });

  const out = new Map<string, MetricCollectionResult>();
  const chunkMaps = new Map<
    string,
    Array<Map<string, NormalizedMetricResult>>
  >();
  requests.forEach(({ key, active }, index) => {
    const query = results[index];
    if (!query) return;
    const maps = chunkMaps.get(key) ?? [];
    maps.push(normalizeMetricResults(query.data?.metrics));
    chunkMaps.set(key, maps);
    const existing = out.get(key);
    // Same guard as the single-collection hook: a disabled chunk has no valid
    // request to send, and `refetch()` would send it anyway.
    const refetch = () => {
      if (active) void query.refetch();
    };
    out.set(key, {
      byKey: new Map(),
      previousByKey: null,
      // Pending covers the catalog wait too — otherwise a screen reads "no
      // data" for the moment before its requests are even allowed to fire.
      isPending:
        (existing?.isPending ?? false) ||
        (query.isPending && active) ||
        (entitySelected(entity, ids) && catalog.isPending),
      isFetching: (existing?.isFetching ?? false) || query.isFetching,
      isError: (existing?.isError ?? false) || (active && query.isError),
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
    if (!entry) continue;
    const served = mergeNormalizedResults(maps);
    entry.byKey = projectPrimary(served);
    entry.previousByKey = compareTo ? projectComparison(served) : null;
  }
  return out;
}
