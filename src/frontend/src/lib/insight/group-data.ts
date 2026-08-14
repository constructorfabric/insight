import type { MetricGroup } from "@/lib/insight/groups";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { derivePeerStanding } from "@/lib/metrics/peer-standing";
import type { PeerStatusWithNeutral } from "@/lib/peers";

/**
 * Whether a section has anything to say about this person in this period.
 *
 * The same test the card applies to decide it is empty, lifted out so the
 * screen above can apply it FIRST. A card that says "No data" is a
 * full-height box carrying one sentence; three of them took a third of a
 * person page and pushed what the person actually does below the fold.
 */
export function groupHasData(
  def: MetricGroup,
  byKey: Map<string, NormalizedMetricResult>,
  entityId: string,
): boolean {
  return def.collection.metrics.some((m) => {
    const metric = byKey.get(m.key);
    return metric != null && forEntity(metric, entityId).value != null;
  });
}


/** Worst standing first — the row a card leads with is the one to look at. */
const HEADLINE_TIER: Record<PeerStatusWithNeutral, number> = {
  bottom: 3,
  in_pack: 2,
  top: 1,
  neutral: 0,
};

/**
 * The metric a section card states in its summary line, or null when it has
 * nothing to say.
 *
 * Shared so the attention block can leave it alone: the card's lead is the
 * most prominent thing on it, and repeating it above was the same finding
 * twice — the duplication that makes a handful of findings read as twice as
 * many. Excluding only `card.preview` misses this one, because the lead is
 * chosen from EVERY metric of the group, not from the keys the card lists.
 */
export function groupHeadlineKey(
  def: MetricGroup,
  byKey: Map<string, NormalizedMetricResult>,
  entityId: string,
): string | null {
  let best: { key: string; tier: number; severity: number } | null = null;
  let firstWithValue: string | null = null;
  for (const m of def.collection.metrics) {
    const metric = byKey.get(m.key);
    if (!metric) continue;
    const data = forEntity(metric, entityId);
    if (data.value != null && firstWithValue == null) firstWithValue = m.key;
    const standing = derivePeerStanding(metric.direction, {
      value: data.value,
      peer: metric.peer?.values.find((v) => v.entity_id === entityId) ?? null,
    });
    if (standing.rank === "neutral") continue;
    const candidate = {
      key: m.key,
      tier: HEADLINE_TIER[standing.rank],
      severity: standing.severity,
    };
    if (
      !best ||
      candidate.tier > best.tier ||
      (candidate.tier === best.tier && candidate.severity > best.severity)
    ) {
      best = candidate;
    }
  }
  return best?.key ?? firstWithValue;
}
