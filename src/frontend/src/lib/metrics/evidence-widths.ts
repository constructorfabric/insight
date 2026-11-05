/** How wide a column would like to be, in px, and whether it takes spare room. */
export interface ColumnNeed {
  natural: number;
  stretches: boolean;
}

/**
 * Lays columns out at their natural widths, never narrower.
 *
 * A row that fits leaves spare room, which goes to the stretching columns in
 * proportion to their widths, or to every column when none stretches. A row
 * that does not fit keeps every width, so the table scrolls sideways instead
 * of cutting one column to make room for another.
 */
export function fitWidths(
  needs: readonly ColumnNeed[],
  available: number
): number[] {
  const natural = needs.map((need) => need.natural);
  const spare = available - natural.reduce((sum, width) => sum + width, 0);
  if (spare <= 0) return natural;

  const growers = needs.some((need) => need.stretches)
    ? needs.map((need) => need.stretches)
    : needs.map(() => true);
  const growing = natural.reduce(
    (sum, width, index) => (growers[index] ? sum + width : sum),
    0
  );
  const count = growers.filter(Boolean).length;

  return natural.map((width, index) => {
    if (!growers[index]) return width;
    const share = growing === 0 ? 1 / count : width / growing;
    return width + spare * share;
  });
}
