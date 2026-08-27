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
  /**
   * The dimension VALUE the row is keyed by — the id the response grouped on.
   * Two rows may share a label; they never share this, which is why React
   * keys and click narrowing both read it rather than the label.
   */
  key: string;
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

/**
 * Segment order for a split whose values mean something in sequence.
 *
 * Sorting a pair like default/non-default by size makes the same reading swap
 * sides between bars, and a reader then compares the wrong halves. A split
 * listed here keeps its declared order on every bar; anything else stays
 * sorted by size, where the largest segment leading is the useful order.
 */
const SEGMENT_ORDER: Record<string, readonly string[]> = {
  branch_scope: ["default", "non_default"],
};

function segmentsOf(
  split: Map<string, BarSegment>,
  splitBy: string | undefined
): BarSegment[] {
  const declared = splitBy ? SEGMENT_ORDER[splitBy] : undefined;
  const segments = [...split.values()];
  if (!declared) return segments.sort((a, b) => b.value - a.value);
  const rank = (seed: string) => {
    const at = declared.indexOf(seed);
    return at === -1 ? declared.length : at;
  };
  return segments.sort((a, b) => rank(a.seed) - rank(b.seed) || b.value - a.value);
}

export function toBarRows(
  bucket: Map<string, BarEntry>,
  splitBy?: string
): BarRow[] {
  const total =
    [...bucket.values()].reduce((sum, entry) => sum + entry.value, 0) || 1;
  return [...bucket.entries()]
    .map(([key, { label, value, split, href }]) => ({
      key,
      label,
      value,
      href,
      segments: split ? segmentsOf(split, splitBy) : undefined,
      // Exact share; rounding happens where it is displayed, so a small one
      // can say so instead of being flattened to zero.
      pct: (value / total) * 100,
    }))
    .sort((a, b) => b.value - a.value);
}

/**
 * The label a response gave one dimension value, from whichever view named it.
 *
 * A URL carries the value — `<source>:<owner>/<repo>` — because that is the id
 * two look-alike rows are told apart by. What a reader recognises is the label,
 * and only the rows know it, so a screen about one value has to look it up
 * rather than parse the id.
 */
export function labelForDimensionValue(
  dimension: string | null,
  value: string,
  ...sources: ReadonlyArray<
    ReadonlyMap<
      string,
      {
        breakdown?: { values: readonly { dimensions: readonly DimensionRef[] }[] };
        rollup?: { values: readonly { dimensions: readonly DimensionRef[] }[] };
      }
    >
  >
): string | undefined {
  if (!dimension) return undefined;
  for (const byKey of sources) {
    for (const result of byKey.values()) {
      for (const view of [result.breakdown, result.rollup]) {
        for (const row of view?.values ?? []) {
          const dim = row.dimensions.find((d) => d.key === dimension);
          if (dim?.value === value && dim.label?.trim()) return dim.label.trim();
        }
      }
    }
  }
  return undefined;
}

interface DimensionRef {
  key: string;
  value: string;
  label?: string | null;
}
