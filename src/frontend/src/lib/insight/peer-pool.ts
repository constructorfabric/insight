import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

/**
 * How many people the medians on this page were computed against.
 *
 * Every comparison on a person page reads "vs median" without ever saying
 * whose median, which leaves the reader unable to judge any of them — and it
 * hides the case that matters most, a lead measured against the developers
 * reporting to them.
 *
 * Pools differ per metric: only people a connector actually measures land in
 * one, so a metric nobody else has yields a smaller pool than the section's.
 * The most common size is reported rather than the largest, so the number
 * describes the typical comparison instead of the most flattering one.
 */
export function typicalPeerPool(
  byKey: Map<string, NormalizedMetricResult>,
  entityId: string,
): number | null {
  const counts = new Map<number, number>();
  for (const metric of byKey.values()) {
    const n = forEntity(metric, entityId).peer?.n;
    if (typeof n !== "number" || n <= 0) continue;
    counts.set(n, (counts.get(n) ?? 0) + 1);
  }
  let best: { n: number; seen: number } | null = null;
  for (const [n, seen] of counts) {
    if (!best || seen > best.seen || (seen === best.seen && n > best.n)) {
      best = { n, seen };
    }
  }
  return best?.n ?? null;
}
