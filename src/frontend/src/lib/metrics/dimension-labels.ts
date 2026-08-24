/**
 * Display vocabulary for dimension KEYS.
 *
 * `/v1/metric-results` labels dimension VALUES — a `branch_scope` row arrives
 * as `{ key: "branch_scope", value: "default", label: "Default branch" }` — but
 * it never labels the key itself, and `/v1/metric-definitions` lists dimensions
 * as bare strings. So the key's wording lives here, and every renderer that
 * shows a dimension name reads it from one place.
 */

/** Standalone label, for a control or a chip: `branch_scope` → `Branch scope`. */
export function dimensionName(dimension: string): string {
  const label = dimension.replaceAll("_", " ");
  return label.charAt(0).toUpperCase() + label.slice(1);
}

/** The same words mid-sentence: `Daily by destination branch`. */
export function dimensionDescription(dimension: string): string {
  return dimension.replaceAll("_", " ");
}

/**
 * Curated breakdown headings, keyed by dimension. An entry replaces the WHOLE
 * heading rather than the tail of `By …`, so it can name the question the split
 * answers instead of the field it reads.
 */
const BREAKDOWN_HEADINGS: Readonly<Record<string, string>> = {
  branch_scope: "Default branch vs other",
};

/**
 * Heading for a breakdown over `dimensions` — a summary card's split section or
 * a grouped table. Curated wording wins for a single dimension; anything else
 * falls back to the humanised key, which is what keeps a dimension added later
 * readable without an entry here.
 *
 * SAFETY: `Object.hasOwn`, not `in` — `in` reaches Object.prototype, so a
 * dimension named `constructor` or `toString` would return a function where a
 * heading belongs.
 */
export function breakdownHeading(dimensions: readonly string[]): string {
  const [only] = dimensions;
  if (dimensions.length === 1 && only && Object.hasOwn(BREAKDOWN_HEADINGS, only)) {
    return BREAKDOWN_HEADINGS[only];
  }
  return `By ${dimensions.map(dimensionDescription).join(" / ")}`;
}
