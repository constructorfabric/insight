import type { MetricGroup } from "@/lib/insight/groups";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import {
  buildPeerStoryEntries,
  type PeerStoryEntry,
} from "@/lib/metrics/peer-story";
import { derivePeerStanding } from "@/lib/metrics/peer-standing";
import type { PeerStatusWithNeutral } from "@/lib/peers";

/**
 * One member's standing on one collection metric vs their OWN cohort (the
 * peer view resolves each entity against its own org unit). `null` when the
 * pair can't be scored — the shared standing layer's ineligible reasons:
 * neutral direction, missing value, an unmeasured member (null peer target),
 * a suppressed thin cohort, or a flat pool that ranks no one.
 */
export function memberMetricStanding(
  metric: NormalizedMetricResult,
  entityId: string,
): PeerStatusWithNeutral | null {
  const data = forEntity(metric, entityId);
  const standing = derivePeerStanding(metric.direction, {
    value: data.value,
    peer: data.peer,
  });
  return standing.eligible ? standing.rank : null;
}

export interface TeamMetricStanding {
  metric: NormalizedMetricResult;
  top: number;
  inPack: number;
  bottom: number;
  scored: number;
  /**
   * Plurality rank across the roster: more members below their own cohort
   * than in any other band → `bottom`; more above → `top`; a tie or an
   * on-par majority → `in_pack` (no plurality means no pattern); none
   * scorable → `neutral`.
   */
  verdict: PeerStatusWithNeutral;
}

/** Roster rollup per collection metric. */
export function teamMetricStandings(
  def: MetricGroup,
  byKey: Map<string, NormalizedMetricResult>,
  memberIds: string[],
): TeamMetricStanding[] {
  return def.collection.metrics.flatMap((metricConfig) => {
    const metric = byKey.get(metricConfig.key);
    if (!metric) return [];
    let top = 0;
    let inPack = 0;
    let bottom = 0;
    for (const memberId of memberIds) {
      const standing = memberMetricStanding(metric, memberId);
      if (standing === "top") top += 1;
      else if (standing === "bottom") bottom += 1;
      else if (standing === "in_pack") inPack += 1;
    }
    const scored = top + inPack + bottom;
    const verdict: PeerStatusWithNeutral =
      scored === 0
        ? "neutral"
        : bottom > top && bottom > inPack
          ? "bottom"
          : top > bottom && top > inPack
            ? "top"
            : "in_pack";
    return [{ metric, top, inPack, bottom, scored, verdict }];
  });
}

/**
 * Per-person entries across every metrics group, keyed by member id — the
 * source for the heatmap's member details sheet. `byGroup` resolves a group id
 * to its fetched result map; groups still loading resolve undefined and
 * contribute nothing.
 */
export function memberMetricEntries(
  defs: readonly MetricGroup[],
  byGroup: (groupId: string) => Map<string, NormalizedMetricResult> | undefined,
  memberIds: string[],
): Map<string, PeerStoryEntry[]> {
  const out = new Map<string, PeerStoryEntry[]>();
  for (const memberId of memberIds) {
    const entries = defs.flatMap((def) => {
      const byKey = byGroup(def.id);
      return byKey
        ? buildPeerStoryEntries(def.collection, byKey, memberId)
        : [];
    });
    if (entries.length > 0) out.set(memberId, entries);
  }
  return out;
}

/**
 * Per-member bottom-quartile counts across the collection — feeds the
 * "members needing attention" tally.
 */
export function metricBelowCounts(
  def: MetricGroup,
  byKey: Map<string, NormalizedMetricResult>,
  memberIds: string[],
): Map<string, number> {
  const out = new Map<string, number>();
  for (const memberId of memberIds) {
    let below = 0;
    for (const metricConfig of def.collection.metrics) {
      const metric = byKey.get(metricConfig.key);
      if (!metric) continue;
      if (memberMetricStanding(metric, memberId) === "bottom") below += 1;
    }
    if (below > 0) out.set(memberId, below);
  }
  return out;
}
