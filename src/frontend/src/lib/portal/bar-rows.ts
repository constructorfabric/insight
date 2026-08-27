/**
 * The bar-list data model, shared by the person-grain composition sections
 * (DomainLensView) and the tenant lens: rows accumulate into BarEntry buckets,
 * toBarRows measures them against each other, BarList draws the result.
 */

export interface BarSegment {
  /** The split value's own key — the colour seed, stable across rows. */
  seed: string;
  label: string;
  value: number;
}

export interface BarRow {
  label: string;
  value: number;
  pct: number;
  /** Where the row's subject lives, when that is knowable. */
  href?: string;
  /** Absent when the section declares no `splitBy`. */
  segments?: BarSegment[];
}

/**
 * One bar before it is measured against the others.
 *
 * The map key is the row's IDENTITY and this is what the reader sees. They were
 * the same string once, which is why a dimension whose ids share a prefix drew a
 * column of bars all reading alike.
 */
export interface BarEntry {
  label: string;
  value: number;
  href?: string;
  /** Totals per split value, keyed by that value. */
  split?: Map<string, BarSegment>;
}

/** Segment a split row falls in when the response named no split value. */
export const UNSPLIT_SEGMENT = "unsplit";

export function toBarRows(bucket: Map<string, BarEntry>): BarRow[] {
  const total =
    [...bucket.values()].reduce((sum, entry) => sum + entry.value, 0) || 1;
  return [...bucket.values()]
    .map(({ label, value, split, href }) => ({
      label,
      value,
      href,
      segments: split
        ? [...split.values()].sort((a, b) => b.value - a.value)
        : undefined,
      // Exact share; rounding happens where it is displayed, so a small one
      // can say so instead of being flattened to zero.
      pct: (value / total) * 100,
    }))
    .sort((a, b) => b.value - a.value);
}
