/** How wide a column would like to be, and how narrow it may get, in px. */
export interface ColumnNeed {
  natural: number;
  minimum: number;
}

/**
 * The width that fits nine values in ten: one outlier then cannot claim the
 * row, and the rest still show whole.
 */
export function typicalWidth(widths: readonly number[]): number {
  if (widths.length === 0) return 0;
  const sorted = [...widths].sort((a, b) => a - b);
  return sorted[Math.ceil(sorted.length * 0.9) - 1]!;
}

/**
 * Shares `available` px out among columns.
 *
 * Every column fits at its natural width when the row holds them all, and the
 * spare goes out in proportion to those widths. When it does not, only the
 * widest give way: they are cut to one shared width, the one at which the row
 * just fits, so a short column never truncates to make room for a long one.
 * No column goes below its minimum; past that point the row is wider than
 * `available` and the table scrolls.
 */
export function fairWidths(
  needs: readonly ColumnNeed[],
  available: number
): number[] {
  const natural = needs.map((need) => Math.max(need.natural, need.minimum));
  const wanted = natural.reduce((sum, width) => sum + width, 0);
  if (wanted <= available) {
    const spare = available - wanted;
    return natural.map((width) =>
      wanted === 0 ? 0 : width + (spare * width) / wanted
    );
  }

  const leastWidth = needs.reduce((sum, need) => sum + need.minimum, 0);
  if (leastWidth >= available) return needs.map((need) => need.minimum);

  const widthsAt = (cap: number) =>
    needs.map((need, index) =>
      Math.max(need.minimum, Math.min(natural[index]!, cap))
    );
  const totalAt = (cap: number) =>
    widthsAt(cap).reduce((sum, width) => sum + width, 0);

  let low = 0;
  let high = Math.max(...natural);
  for (let step = 0; step < 40; step += 1) {
    const cap = (low + high) / 2;
    if (totalAt(cap) > available) high = cap;
    else low = cap;
  }
  return widthsAt(low);
}
