import type { MetricFormat } from "@/api/metric-results-client";
import {
  forEntity,
  type EntityMetricData,
  type MetricCollectionConfig,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { derivePeerStanding } from "@/lib/metrics/peer-standing";
import type { FocusMode, PeerStats, PeerStatusWithNeutral } from "@/lib/peers";
import {
  evidenceSelection,
  type MetricEvidenceSelection,
} from "@/api/metric-drilldown-client";

/**
 * The peer story ranks a collection's metrics by how far the person sits
 * from their cohort: outliers (top/bottom quartile) ordered by severity,
 * everything else folded as supporting. Pure data — layout lives in
 * `components/widgets/metric-views/peer-story.tsx`.
 */
export type PeerStoryEntry = {
  key: string;
  label: string;
  sublabel?: string;
  value: number;
  unit: string | null;
  format: MetricFormat;
  higherIsBetter: boolean;
  neutral: boolean;
  /** False when the person has no observations (zero-filled own total). */
  observed: boolean;
  stats: PeerStats | null;
  status: PeerStatusWithNeutral;
  gapPct: number | null;
  gapDelta: number;
  severity: number;
  evidence?: MetricEvidenceSelection | null;
};

function toStoryEntry(
  metric: NormalizedMetricResult,
  value: number,
  data: Pick<EntityMetricData, "value" | "peer">,
  entityId: string
): PeerStoryEntry {
  const standing = derivePeerStanding(metric.direction, data);
  return {
    key: metric.metric_key,
    label: metric.label,
    sublabel: metric.description,
    value,
    unit: metric.unit,
    format: metric.format,
    higherIsBetter: metric.direction !== "lower_is_better",
    neutral: metric.direction === "neutral",
    observed: standing.observed,
    stats: standing.stats,
    status: standing.rank,
    gapPct: standing.gapPct,
    gapDelta: standing.gapDelta,
    severity: standing.severity,
    evidence: metric.drilldown
      ? evidenceSelection(metric.selection, entityId)
      : null,
  };
}

/**
 * Entries in collection order; metrics without a period value drop out.
 * All eligibility (observed / suppressed / flat pool / neutral direction)
 * and both judgments come from `derivePeerStanding` — the story only lays
 * the results out.
 */
export function buildPeerStoryEntries(
  collection: MetricCollectionConfig,
  byKey: Map<string, NormalizedMetricResult>,
  entityId: string
): PeerStoryEntry[] {
  return collection.metrics.flatMap((metricConfig) => {
    const metric = byKey.get(metricConfig.key);
    if (!metric) return [];
    const data = forEntity(metric, entityId);
    if (data.value == null || !Number.isFinite(data.value)) return [];
    return [toStoryEntry(metric, data.value, data, entityId)];
  });
}

/**
 * The most severe bottom-quartile entry — a person's headline weak spot, or
 * `null` when nothing ranks below the pack. Distinct from a peer-story hero,
 * which can be a top-quartile strength; this is always a weakness.
 */
export function worstEntry(entries: PeerStoryEntry[]): PeerStoryEntry | null {
  return (
    entries
      .filter((entry) => entry.status === "bottom")
      .sort((a, b) => b.severity - a.severity)[0] ?? null
  );
}

export interface PeerStoryPartition {
  /** Outliers under the active focus mode, severity-ranked. */
  focusedOutliers: PeerStoryEntry[];
  hero: PeerStoryEntry | null;
  sideCards: PeerStoryEntry[];
  chips: PeerStoryEntry[];
  folded: PeerStoryEntry[];
}

export function partitionPeerStory(
  entries: PeerStoryEntry[],
  focusMode: FocusMode
): PeerStoryPartition {
  const bottom = entries
    .filter((entry) => entry.status === "bottom")
    .sort((a, b) => b.severity - a.severity);
  const top = entries
    .filter((entry) => entry.status === "top")
    .sort((a, b) => b.severity - a.severity);
  const folded = entries
    .filter((entry) => entry.status === "in_pack" || entry.status === "neutral")
    .sort((a, b) => {
      if (a.status === b.status) return 0;
      return a.status === "neutral" ? 1 : -1;
    });

  const focusedOutliers =
    focusMode === "critical"
      ? bottom
      : focusMode === "rewards"
        ? top
        : focusMode === "all"
          ? bottom.length > 0
            ? [...bottom, ...top]
            : top
          : [];
  const hero = focusedOutliers[0] ?? null;

  return {
    focusedOutliers,
    hero,
    sideCards: hero ? focusedOutliers.slice(1, 4) : [],
    chips: hero ? focusedOutliers.slice(4) : [],
    folded,
  };
}
