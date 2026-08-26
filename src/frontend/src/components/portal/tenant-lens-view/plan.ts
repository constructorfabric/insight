import type { MetricBucket, MetricGroupLimit } from "@/api/metric-results-client";
import type {
  MetricCollectionConfig,
  NormalizedMetricResult,
} from "@/lib/metrics/collection";
import type { TenantLensConfig, TenantSectionSpec } from "@/lib/portal/lens-configs";

/** How a renderer reaches the result the planner requested for its need. */
export type ResolveView = (
  need: TenantViewNeed
) => NormalizedMetricResult | undefined;

/**
 * Turns a tenant lens config into the requests it needs. The backend rejects
 * a second view of the same kind on one metric within one request
 * (`validation.rs`: "duplicate view"), so a lens whose sections need, say,
 * three differently-dimensioned timeseries of ci.runs cannot ride one
 * request — this planner packs the conflicting views into as few extra
 * collections as possible and remembers where each section's view landed.
 *
 * Both the planner and the section renderers derive a section's needs from
 * the same `sectionNeeds`, so a renderer always finds the view the planner
 * requested for it — there is no second bookkeeping to drift.
 */

export type HalfWindow = "first-half" | "second-half";

export type TenantViewNeed =
  | { view: "period"; metric: string }
  | { view: "histogram"; metric: string }
  | {
      view: "timeseries";
      metric: string;
      bucket: MetricBucket;
      dimensions?: readonly string[];
      groupLimit?: MetricGroupLimit;
    }
  | {
      view: "breakdown";
      metric: string;
      dimensions: readonly string[];
      /**
       * A mergeable need tolerates EXTRA dimensions in the served rows and
       * re-aggregates by summing — only valid for count/sum readings. An
       * exact need (the default) must be served grouped by exactly its
       * dimensions: rates and medians cannot be re-aggregated client-side.
       */
      merge?: boolean;
      /** Served over one half of the window instead of the whole period. */
      window?: HalfWindow;
    };

/** Groups a ranked dimension can spread into before the panel stops reading. */
const MAX_GROUPS = 40;
const SMALL_MULTIPLES_DEFAULT = 12;

/** The views one section draws from, in stable order. */
export function sectionNeeds(
  section: TenantSectionSpec,
  bucket: MetricBucket
): TenantViewNeed[] {
  switch (section.kind) {
    case "headline":
    case "stat-tiles":
      return section.metrics.map((metric) => ({ view: "period", metric }));
    case "trend":
      return section.metrics.map((metric) => ({
        view: "timeseries",
        metric,
        bucket,
      }));
    case "composition":
      return [
        {
          view: "breakdown",
          metric: section.metric,
          dimensions: [section.dimension, ...(section.splitBy ? [section.splitBy] : [])],
          merge: true,
        },
      ];
    case "histogram":
      return [{ view: "histogram", metric: section.metric }];
    case "stacked-trend":
      return [
        {
          view: "timeseries",
          metric: section.metric,
          bucket,
          dimensions: [section.splitBy],
        },
      ];
    case "small-multiples":
      return [
        {
          view: "timeseries",
          metric: section.metric,
          bucket,
          dimensions: [section.dimension],
          groupLimit: {
            count: Math.min(section.top ?? SMALL_MULTIPLES_DEFAULT, MAX_GROUPS),
            include_remainder: false,
          },
        },
      ];
    case "scatter":
      return [
        { view: "breakdown", metric: section.x, dimensions: [section.dimension] },
        { view: "breakdown", metric: section.y, dimensions: [section.dimension] },
        ...(section.size
          ? [
              {
                view: "breakdown",
                metric: section.size,
                dimensions: [section.dimension],
              } as const,
            ]
          : []),
      ];
    case "heatmap-hours":
      // Day buckets regardless of the lens bucket: the weekday axis needs
      // the date, and 400 days × 12 blocks still fits the backend's limits.
      return [
        {
          view: "timeseries",
          metric: section.metric,
          bucket: "day",
          dimensions: ["hour_block"],
        },
      ];
    case "hour-columns":
      return [
        { view: "breakdown", metric: section.metric, dimensions: ["hour_block"] },
      ];
    case "slope":
    case "momentum":
      return [
        {
          view: "breakdown",
          metric: section.metric,
          dimensions: [section.dimension],
          window: "first-half",
        },
        {
          view: "breakdown",
          metric: section.metric,
          dimensions: [section.dimension],
          window: "second-half",
        },
      ];
    case "marginal-impact":
      // Derived gate math over run counts — the dims the gate definition needs.
      return [
        {
          view: "breakdown",
          metric: "ci.runs",
          dimensions: ["pipeline", "trigger", "outcome"],
          merge: true,
        },
      ];
    case "callout-pair":
      return [
        { view: "period", metric: section.metric },
        { view: "breakdown", metric: section.metric, dimensions: [section.dimension] },
      ];
    case "dumbbell":
      return [
        {
          view: "breakdown",
          metric: section.metric,
          dimensions: [section.dimension, section.splitBy],
        },
      ];
    case "cumulative":
      return [
        {
          view: "breakdown",
          metric: section.metric,
          dimensions: [section.dimension],
          merge: true,
        },
      ];
    case "decomposition":
      return [
        {
          view: "breakdown",
          metric: section.metric,
          dimensions: [section.splitBy],
          merge: true,
        },
      ];
    case "verdict-table":
      // Weekly regardless of the lens bucket: the verdict IS about weekly
      // spread, so a monthly lens must not coarsen it away.
      return [
        {
          view: "timeseries",
          metric: section.metric,
          bucket: "week",
          dimensions: [section.dimension],
          groupLimit: { count: MAX_GROUPS, include_remainder: false },
        },
      ];
    default: {
      const _exhaustive: never = section;
      throw new Error(`Unhandled tenant section: ${JSON.stringify(_exhaustive)}`);
    }
  }
}

export type NeedLocation =
  | { at: "collection"; index: number }
  | { at: "first-half" }
  | { at: "second-half" };

export interface TenantRequestPlan {
  /** Concurrent whole-period requests; index 0 carries every period view. */
  collections: MetricCollectionConfig[];
  /** Views served over each half of the window (empty when unused). */
  halves: MetricCollectionConfig;
  locate(need: TenantViewNeed): NeedLocation | undefined;
}

interface Slot {
  view: "period" | "histogram" | "timeseries" | "breakdown";
  metric: string;
  bucket?: MetricBucket;
  dimensions?: string[];
  groupLimit?: MetricGroupLimit;
  merge?: boolean;
  halves?: boolean;
}

function needSignature(need: TenantViewNeed): string {
  switch (need.view) {
    case "period":
    case "histogram":
      return `${need.view}|${need.metric}`;
    case "timeseries":
      return [
        "timeseries",
        need.metric,
        need.bucket,
        [...(need.dimensions ?? [])].sort().join(","),
        need.groupLimit?.count ?? "",
        need.groupLimit?.include_remainder ?? "",
      ].join("|");
    case "breakdown":
      return [
        "breakdown",
        need.metric,
        [...need.dimensions].sort().join(","),
        need.merge ? "merge" : "exact",
        need.window ?? "",
      ].join("|");
  }
}

// The merge flag stays in the identity while slotting: an exact consumer
// must never share a slot a later merge need could widen under it. The
// post-widening coalesce pass drops the flag and merges true duplicates.
function slotSignature(slot: Slot): string {
  return [
    slot.view,
    slot.metric,
    slot.bucket ?? "",
    [...(slot.dimensions ?? [])].sort().join(","),
    slot.groupLimit?.count ?? "",
    slot.groupLimit?.include_remainder ?? "",
    slot.merge ? "merge" : "",
    slot.halves ? "halves" : "",
  ].join("|");
}

export function planTenantRequests(
  config: TenantLensConfig,
  bucket: MetricBucket
): TenantRequestPlan {
  const slots: Slot[] = [];
  const consumerSlot = new Map<string, number>();

  const findSlot = (candidate: Slot) =>
    slots.findIndex((slot) => slotSignature(slot) === slotSignature(candidate));

  for (const section of config.sections) {
    for (const need of sectionNeeds(section, bucket)) {
      const sig = needSignature(need);
      if (consumerSlot.has(sig)) continue;
      if (need.view === "breakdown" && need.merge && !need.window) {
        const dims = new Set(need.dimensions);
        // A merge consumer re-aggregates, so any slot whose dims cover its
        // own serves it; widening a narrower merge slot keeps that true for
        // the slot's earlier consumers too.
        let index = slots.findIndex(
          (slot) =>
            slot.view === "breakdown" &&
            slot.metric === need.metric &&
            slot.merge === true &&
            !slot.halves &&
            [...dims].every((d) => slot.dimensions?.includes(d))
        );
        if (index < 0) {
          index = slots.findIndex(
            (slot) =>
              slot.view === "breakdown" &&
              slot.metric === need.metric &&
              slot.merge === true &&
              !slot.halves &&
              (slot.dimensions ?? []).every((d) => dims.has(d))
          );
          if (index >= 0) {
            slots[index].dimensions = [
              ...new Set([...(slots[index].dimensions ?? []), ...need.dimensions]),
            ];
          }
        }
        if (index < 0) {
          index =
            slots.push({
              view: "breakdown",
              metric: need.metric,
              dimensions: [...need.dimensions],
              merge: true,
            }) - 1;
        }
        consumerSlot.set(sig, index);
        continue;
      }
      const candidate: Slot =
        need.view === "period" || need.view === "histogram"
          ? { view: need.view, metric: need.metric }
          : need.view === "timeseries"
            ? {
                view: "timeseries",
                metric: need.metric,
                bucket: need.bucket,
                dimensions: need.dimensions ? [...need.dimensions] : undefined,
                groupLimit: need.groupLimit,
              }
            : {
                view: "breakdown",
                metric: need.metric,
                dimensions: [...need.dimensions],
                merge: false,
                halves: Boolean(need.window),
              };
      let index = findSlot(candidate);
      if (index < 0) index = slots.push(candidate) - 1;
      consumerSlot.set(sig, index);
    }
  }

  // A widened merge slot can end up identical to an exact slot with the same
  // dims — serve both consumers from one request instead of two.
  const canonical = new Map<string, number>();
  const remap = new Map<number, number>();
  slots.forEach((slot, index) => {
    const key = [
      slot.view,
      slot.metric,
      slot.bucket ?? "",
      [...(slot.dimensions ?? [])].sort().join(","),
      slot.groupLimit?.count ?? "",
      slot.groupLimit?.include_remainder ?? "",
      slot.halves ? "halves" : "",
    ].join("|");
    const kept = canonical.get(key);
    if (kept == null) canonical.set(key, index);
    remap.set(index, kept ?? index);
  });

  // First-fit pack: a collection can hold one view of each kind per metric.
  // Periods lead so they all land in collection 0 — only the primary request
  // carries the previous-period twin the tiles derive deltas from.
  const orderedSlots = [...new Set(remap.values())].sort((a, b) => {
    const rank = (i: number) => (slots[i].view === "period" ? 0 : 1);
    return rank(a) - rank(b) || a - b;
  });
  const collections: number[][] = [];
  const halves: number[] = [];
  const placement = new Map<number, NeedLocation | "halves">();
  for (const index of orderedSlots) {
    const slot = slots[index];
    if (slot.halves) {
      const conflict = halves.some(
        (other) => slots[other].metric === slot.metric && slots[other].view === slot.view
      );
      if (conflict) {
        throw new Error(
          `Tenant lens config needs two different ${slot.view} views of ${slot.metric} per half-window — not representable in one halves request`
        );
      }
      halves.push(index);
      placement.set(index, "halves");
      continue;
    }
    let target = collections.findIndex(
      (members) =>
        !members.some(
          (other) => slots[other].metric === slot.metric && slots[other].view === slot.view
        )
    );
    if (target < 0) target = collections.push([]) - 1;
    collections[target].push(index);
    placement.set(index, { at: "collection", index: target });
  }

  const toConfig = (members: number[]): MetricCollectionConfig => {
    const byMetric = new Map<string, number[]>();
    for (const index of members) {
      const got = byMetric.get(slots[index].metric) ?? [];
      got.push(index);
      byMetric.set(slots[index].metric, got);
    }
    return {
      metrics: [...byMetric.entries()].map(([key, indexes]) => ({
        key,
        views: indexes.map((index) => {
          const slot = slots[index];
          switch (slot.view) {
            case "period":
              return { view: "period" as const };
            case "histogram":
              return { view: "histogram" as const };
            case "timeseries":
              return {
                view: "timeseries" as const,
                bucket: slot.bucket ?? "auto",
                ...(slot.dimensions ? { dimensions: slot.dimensions } : {}),
                ...(slot.groupLimit ? { groupLimit: slot.groupLimit } : {}),
              };
            case "breakdown":
              return {
                view: "breakdown" as const,
                dimensions: slot.dimensions ?? [],
              };
          }
        }),
      })),
    };
  };

  return {
    collections: collections.length ? collections.map(toConfig) : [{ metrics: [] }],
    halves: toConfig(halves),
    locate(need) {
      const raw = consumerSlot.get(needSignature(need));
      if (raw == null) return undefined;
      const index = remap.get(raw) ?? raw;
      const placed = placement.get(index);
      if (placed == null) return undefined;
      if (placed === "halves") {
        return need.view === "breakdown" && need.window
          ? { at: need.window }
          : undefined;
      }
      return placed;
    },
  };
}
