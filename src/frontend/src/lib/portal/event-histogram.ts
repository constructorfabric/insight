import { forEntity, type NormalizedMetricResult } from "@/lib/metrics/collection";

/** One merged histogram bar: a half-open [lo, hi) band and how many events fell in it. */
export interface EventBin {
  lo: number;
  hi: number;
  count: number;
}

/**
 * Merge per-entity server histograms into one org event histogram — valid
 * only when every entity that HAS bins shares identical bin edges (design §7
 * open question). Returns null when edges differ, when an entity has an
 * anomalous bin count, or when nobody has bins at all; the caller falls back
 * honestly (no chart) rather than summing incomparable bins.
 *
 * A member with no bins is skipped, not a reason to bail: they simply had no
 * events in the period, which is a normal reading on any real roster. Bailing
 * would blank the chart for the whole org whenever one person was inactive.
 */
export function mergeEventHistogram(
  result: NormalizedMetricResult | undefined,
  memberIds: readonly string[],
): EventBin[] | null {
  if (!result?.histogram) return null;
  let reference: EventBin[] | null = null;
  const totals: number[] = [];
  for (const id of memberIds) {
    const bins = forEntity(result, id).histogram[0]?.bins;
    if (!bins?.length) continue;
    if (!reference) {
      reference = bins.map((b) => ({ lo: b.lo, hi: b.hi, count: 0 }));
      totals.length = bins.length;
      totals.fill(0);
    }
    if (bins.length !== reference.length) return null;
    for (let i = 0; i < bins.length; i++) {
      if (bins[i]!.lo !== reference[i]!.lo || bins[i]!.hi !== reference[i]!.hi) return null;
      totals[i] += bins[i]!.count;
    }
  }
  if (!reference) return null;
  return reference.map((b, i) => ({ ...b, count: totals[i]! }));
}
